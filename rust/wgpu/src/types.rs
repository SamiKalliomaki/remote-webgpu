//! Plain data types mirroring the `wgpu` public API (the subset the wgpu
//! examples exercise), independent of any backend handle.

use std::borrow::Cow;
use std::num::{NonZeroU32, NonZeroU64};

pub use crate::limits::Limits;

pub type BufferAddress = u64;
pub type BufferSize = NonZeroU64;
pub type DynamicOffset = u32;
pub type ShaderLocation = u32;

pub const COPY_BUFFER_ALIGNMENT: BufferAddress = 4;
pub const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = 256;
pub const MAP_ALIGNMENT: BufferAddress = 8;
pub const QUERY_RESOLVE_BUFFER_ALIGNMENT: BufferAddress = 256;
pub const QUERY_SIZE: u32 = 8;
pub const PUSH_CONSTANT_ALIGNMENT: u32 = 4;
pub const VERTEX_STRIDE_ALIGNMENT: BufferAddress = 4;

/// On native the wgpu types are all Send + Sync; ours are too.
pub trait WasmNotSend: Send {}
impl<T: Send> WasmNotSend for T {}
pub trait WasmNotSync: Sync {}
impl<T: Sync> WasmNotSync for T {}
pub trait WasmNotSendSync: Send + Sync {}
impl<T: Send + Sync> WasmNotSendSync for T {}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct Backends: u64 {
        const VULKAN = 1 << 0;
        const GL = 1 << 1;
        const METAL = 1 << 2;
        const DX12 = 1 << 3;
        const BROWSER_WEBGPU = 1 << 4;
        const NOOP = 1 << 5;
        const PRIMARY = Self::VULKAN.bits() | Self::METAL.bits() | Self::DX12.bits() | Self::BROWSER_WEBGPU.bits();
        const SECONDARY = Self::GL.bits();
    }
}

impl Default for Backends {
    fn default() -> Self {
        Backends::all()
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct InstanceFlags: u32 {
        const DEBUG = 1 << 0;
        const VALIDATION = 1 << 1;
        const DISCARD_HAL_LABELS = 1 << 2;
        const GPU_BASED_VALIDATION = 1 << 3;
        const AUTOMATIC_TIMESTAMP_NORMALIZATION = 1 << 4;
    }
}

impl InstanceFlags {
    pub fn advanced_debugging() -> Self {
        Self::DEBUG | Self::VALIDATION | Self::GPU_BASED_VALIDATION
    }
    pub fn from_build_config() -> Self {
        Self::empty()
    }
    pub fn with_env(self) -> Self {
        self
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct Features: u128 {
        const DEPTH_CLIP_CONTROL = 1 << 0;
        const DEPTH32FLOAT_STENCIL8 = 1 << 1;
        const TIMESTAMP_QUERY = 1 << 2;
        const INDIRECT_FIRST_INSTANCE = 1 << 3;
        const SHADER_F16 = 1 << 4;
        const RG11B10UFLOAT_RENDERABLE = 1 << 5;
        const BGRA8UNORM_STORAGE = 1 << 6;
        const FLOAT32_FILTERABLE = 1 << 7;
        const FLOAT32_BLENDABLE = 1 << 8;
        const CLIP_DISTANCES = 1 << 9;
        const DUAL_SOURCE_BLENDING = 1 << 10;
        const SUBGROUPS = 1 << 11;
        const TEXTURE_COMPRESSION_BC = 1 << 12;
        const TEXTURE_COMPRESSION_BC_SLICED_3D = 1 << 13;
        const TEXTURE_COMPRESSION_ETC2 = 1 << 14;
        const TEXTURE_COMPRESSION_ASTC = 1 << 15;
        const TEXTURE_COMPRESSION_ASTC_SLICED_3D = 1 << 16;
        const PRIMITIVE_INDEX = 1 << 17;
        const TEXTURE_FORMATS_TIER1 = 1 << 18;
        const TEXTURE_FORMATS_TIER2 = 1 << 19;
        const TEXTURE_COMPONENT_SWIZZLE = 1 << 20;

        // Native-only features: never reported by the remote adapter, but
        // examples reference the constants.
        const TIMESTAMP_QUERY_INSIDE_ENCODERS = 1 << 32;
        const TIMESTAMP_QUERY_INSIDE_PASSES = 1 << 33;
        const PIPELINE_STATISTICS_QUERY = 1 << 34;
        const MULTIVIEW = 1 << 35;
        const SELECTIVE_MULTIVIEW = 1 << 36;
        const CONSERVATIVE_RASTERIZATION = 1 << 37;
        const POLYGON_MODE_LINE = 1 << 38;
        const POLYGON_MODE_POINT = 1 << 39;
        const TEXTURE_BINDING_ARRAY = 1 << 40;
        const BUFFER_BINDING_ARRAY = 1 << 41;
        const STORAGE_RESOURCE_BINDING_ARRAY = 1 << 42;
        const SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING = 1 << 43;
        const PARTIALLY_BOUND_BINDING_ARRAY = 1 << 44;
        const VERTEX_WRITABLE_STORAGE = 1 << 45;
        const TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES = 1 << 46;
        const ADDRESS_MODE_CLAMP_TO_BORDER = 1 << 47;
        const ADDRESS_MODE_CLAMP_TO_ZERO = 1 << 48;
        const PUSH_CONSTANTS = 1 << 49;
        const IMMEDIATES = 1 << 50;
        const PASSTHROUGH_SHADERS = 1 << 51;
        const MAPPABLE_PRIMARY_BUFFERS = 1 << 52;
        const EXPERIMENTAL_RAY_QUERY = 1 << 64;
        const EXPERIMENTAL_RAY_TRACING_ACCELERATION_STRUCTURE = 1 << 65;
        const EXPERIMENTAL_RAY_HIT_VERTEX_RETURN = 1 << 66;
        const EXPERIMENTAL_MESH_SHADER = 1 << 67;
        const EXPERIMENTAL_MESH_SHADER_MULTIVIEW = 1 << 68;
        const EXPERIMENTAL_COOPERATIVE_MATRIX = 1 << 69;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct DownlevelFlags: u32 {
        const COMPUTE_SHADERS = 1 << 0;
        const FRAGMENT_WRITABLE_STORAGE = 1 << 1;
        const INDIRECT_EXECUTION = 1 << 2;
        const BASE_VERTEX = 1 << 3;
        const READ_ONLY_DEPTH_STENCIL = 1 << 4;
        const NON_POWER_OF_TWO_MIPMAPPED_TEXTURES = 1 << 5;
        const CUBE_ARRAY_TEXTURES = 1 << 6;
        const COMPARISON_SAMPLERS = 1 << 7;
        const INDEPENDENT_BLEND = 1 << 8;
        const VERTEX_STORAGE = 1 << 9;
        const ANISOTROPIC_FILTERING = 1 << 10;
        const FRAGMENT_STORAGE = 1 << 11;
        const MULTISAMPLED_SHADING = 1 << 12;
        const DEPTH_TEXTURE_AND_BUFFER_COPIES = 1 << 13;
        const WEBGPU_TEXTURE_FORMAT_SUPPORT = 1 << 14;
        const BUFFER_BINDINGS_NOT_16_BYTE_ALIGNED = 1 << 15;
        const UNRESTRICTED_INDEX_BUFFER = 1 << 16;
        const FULL_DRAW_INDEX_UINT32 = 1 << 17;
        const DEPTH_BIAS_CLAMP = 1 << 18;
        const VIEW_FORMATS = 1 << 19;
        const UNRESTRICTED_EXTERNAL_TEXTURE_COPIES = 1 << 20;
        const SURFACE_VIEW_FORMATS = 1 << 21;
        const NONBLOCKING_QUERY_RESOLVE = 1 << 22;
    }
}

impl DownlevelFlags {
    pub fn compliant() -> Self {
        Self::all()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShaderModel {
    Sm2,
    Sm4,
    Sm5,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct DownlevelLimits {}

#[allow(clippy::derivable_impls)]
impl Default for DownlevelLimits {
    fn default() -> Self {
        DownlevelLimits {}
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownlevelCapabilities {
    pub flags: DownlevelFlags,
    pub limits: DownlevelLimits,
    pub shader_model: ShaderModel,
}

impl Default for DownlevelCapabilities {
    fn default() -> Self {
        Self {
            flags: DownlevelFlags::all(),
            limits: DownlevelLimits::default(),
            shader_model: ShaderModel::Sm5,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Backend {
    Noop,
    Vulkan,
    Metal,
    Dx12,
    Gl,
    BrowserWebGpu,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum DeviceType {
    Other,
    IntegratedGpu,
    DiscreteGpu,
    VirtualGpu,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdapterInfo {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: DeviceType,
    pub driver: String,
    pub driver_info: String,
    pub backend: Backend,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum PowerPreference {
    #[default]
    None,
    LowPower,
    HighPerformance,
}

impl PowerPreference {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("WGPU_POWER_PREF").ok()?;
        match value.to_lowercase().as_str() {
            "low" => Some(Self::LowPower),
            "high" => Some(Self::HighPerformance),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryBudgetThresholds {
    pub for_resource_creation: Option<u8>,
    pub for_device_loss: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct BackendOptions {}

/// Object-safe stand-in for `raw-window-handle`'s display handle trait; the
/// remote backend has no display, so anything is accepted.
pub trait WgpuHasDisplayHandle {}
impl<T> WgpuHasDisplayHandle for T {}

pub struct InstanceDescriptor {
    pub backends: Backends,
    pub flags: InstanceFlags,
    pub memory_budget_thresholds: MemoryBudgetThresholds,
    pub backend_options: BackendOptions,
    pub display: Option<Box<dyn WgpuHasDisplayHandle>>,
}

impl Default for InstanceDescriptor {
    fn default() -> Self {
        Self::new_without_display_handle()
    }
}

impl InstanceDescriptor {
    pub fn new_without_display_handle() -> Self {
        Self {
            backends: Backends::all(),
            flags: InstanceFlags::default(),
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
            backend_options: BackendOptions::default(),
            display: None,
        }
    }
    pub fn new_with_display_handle(display: Box<dyn WgpuHasDisplayHandle>) -> Self {
        let mut this = Self::new_without_display_handle();
        this.display = Some(display);
        this
    }
    pub fn new_without_display_handle_from_env() -> Self {
        Self::new_without_display_handle()
    }
    pub fn new_with_display_handle_from_env(display: Box<dyn WgpuHasDisplayHandle>) -> Self {
        Self::new_with_display_handle(display)
    }
    pub fn with_env(self) -> Self {
        self
    }
    pub fn with_display_handle(mut self, display: Box<dyn WgpuHasDisplayHandle>) -> Self {
        self.display = Some(display);
        self
    }
}

/// Token acknowledging experimental feature use; accepted and ignored.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct ExperimentalFeatures {
    enabled: bool,
}

impl ExperimentalFeatures {
    pub const fn disabled() -> Self {
        Self { enabled: false }
    }
    /// # Safety
    /// Matches upstream wgpu's contract; the remote backend forwards nothing
    /// experimental, so this is inert.
    pub const unsafe fn enabled() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Default)]
pub enum MemoryHints {
    #[default]
    Performance,
    MemoryUsage,
    Manual {
        suballocated_device_memory_block_size: std::ops::Range<u64>,
    },
}

#[derive(Debug, Clone, Default)]
pub enum Trace {
    #[default]
    Off,
    Directory(std::path::PathBuf),
}

#[derive(Debug, Clone, Default)]
pub struct QueueDescriptor<'a> {
    pub label: Label<'a>,
}

pub type Label<'a> = Option<&'a str>;

#[derive(Clone, Default)]
pub struct DeviceDescriptor<'a> {
    pub label: Label<'a>,
    pub required_features: Features,
    pub required_limits: Limits,
    pub memory_hints: MemoryHints,
    pub experimental_features: ExperimentalFeatures,
    pub trace: Trace,
    pub default_queue: QueueDescriptor<'a>,
}

#[derive(Clone, Default)]
pub struct RequestAdapterOptions<'a, 'b> {
    pub power_preference: PowerPreference,
    pub force_fallback_adapter: bool,
    pub compatible_surface: Option<&'a crate::Surface<'b>>,
}

#[derive(Clone, Debug)]
pub enum RequestAdapterError {
    NotFound,
}
impl std::error::Error for RequestAdapterError {}
impl std::fmt::Display for RequestAdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no suitable adapter found")
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollStatus {
    QueueEmpty,
    WaitSucceeded,
    Poll,
}

impl PollStatus {
    pub fn is_queue_empty(&self) -> bool {
        matches!(self, Self::QueueEmpty)
    }
    pub fn wait_finished(&self) -> bool {
        matches!(self, Self::WaitSucceeded | Self::QueueEmpty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollError {
    Timeout,
}
impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "poll timed out")
    }
}
impl std::error::Error for PollError {}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum PollType<T = SubmissionIndex> {
    Wait {
        submission_index: Option<T>,
        timeout: Option<std::time::Duration>,
    },
    #[default]
    Poll,
}

impl<T> PollType<T> {
    pub const fn wait_indefinitely() -> Self {
        Self::Wait {
            submission_index: None,
            timeout: None,
        }
    }
    pub const fn wait() -> Self {
        Self::wait_indefinitely()
    }
}

#[derive(Debug, Clone)]
pub struct SubmissionIndex(#[allow(dead_code)] pub(crate) u64);

// ---------------------------------------------------------------------
// resource usage flags
// ---------------------------------------------------------------------

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct BufferUsages: u64 {
        const MAP_READ = 1 << 0;
        const MAP_WRITE = 1 << 1;
        const COPY_SRC = 1 << 2;
        const COPY_DST = 1 << 3;
        const INDEX = 1 << 4;
        const VERTEX = 1 << 5;
        const UNIFORM = 1 << 6;
        const STORAGE = 1 << 7;
        const INDIRECT = 1 << 8;
        const QUERY_RESOLVE = 1 << 9;
        const BLAS_INPUT = 1 << 10;
        const TLAS_INPUT = 1 << 11;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct TextureUsages: u64 {
        const COPY_SRC = 1 << 0;
        const COPY_DST = 1 << 1;
        const TEXTURE_BINDING = 1 << 2;
        const STORAGE_BINDING = 1 << 3;
        const RENDER_ATTACHMENT = 1 << 4;
        const TRANSIENT_ATTACHMENT = 1 << 5;
        const STORAGE_ATOMIC = 1 << 6;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct ShaderStages: u32 {
        const NONE = 0;
        const VERTEX = 1 << 0;
        const FRAGMENT = 1 << 1;
        const COMPUTE = 1 << 2;
        const VERTEX_FRAGMENT = Self::VERTEX.bits() | Self::FRAGMENT.bits();
        const TASK = 1 << 3;
        const MESH = 1 << 4;
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct ColorWrites: u32 {
        const RED = 1 << 0;
        const GREEN = 1 << 1;
        const BLUE = 1 << 2;
        const ALPHA = 1 << 3;
        const COLOR = Self::RED.bits() | Self::GREEN.bits() | Self::BLUE.bits();
        const ALL = Self::RED.bits() | Self::GREEN.bits() | Self::BLUE.bits() | Self::ALPHA.bits();
    }
}

impl Default for ColorWrites {
    fn default() -> Self {
        Self::ALL
    }
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct TextureFormatFeatureFlags: u32 {
        const FILTERABLE = 1 << 0;
        const MULTISAMPLE_X2 = 1 << 1;
        const MULTISAMPLE_X4 = 1 << 2;
        const MULTISAMPLE_X8 = 1 << 3;
        const MULTISAMPLE_X16 = 1 << 4;
        const MULTISAMPLE_RESOLVE = 1 << 5;
        const STORAGE_READ_ONLY = 1 << 6;
        const STORAGE_WRITE_ONLY = 1 << 7;
        const STORAGE_READ_WRITE = 1 << 8;
        const STORAGE_ATOMIC = 1 << 9;
        const BLENDABLE = 1 << 10;
    }
}

impl TextureFormatFeatureFlags {
    pub fn sample_count_supported(&self, count: u32) -> bool {
        match count {
            1 => true,
            2 => self.contains(Self::MULTISAMPLE_X2),
            4 => self.contains(Self::MULTISAMPLE_X4),
            8 => self.contains(Self::MULTISAMPLE_X8),
            16 => self.contains(Self::MULTISAMPLE_X16),
            _ => false,
        }
    }
    pub fn supported_sample_counts(&self) -> Vec<u32> {
        [1, 2, 4, 8, 16]
            .into_iter()
            .filter(|&count| self.sample_count_supported(count))
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextureFormatFeatures {
    pub allowed_usages: TextureUsages,
    pub flags: TextureFormatFeatureFlags,
}

bitflags::bitflags! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
    pub struct PipelineStatisticsTypes: u32 {
        const VERTEX_SHADER_INVOCATIONS = 1 << 0;
        const CLIPPER_INVOCATIONS = 1 << 1;
        const CLIPPER_PRIMITIVES_OUT = 1 << 2;
        const FRAGMENT_SHADER_INVOCATIONS = 1 << 3;
        const COMPUTE_SHADER_INVOCATIONS = 1 << 4;
    }
}

// ---------------------------------------------------------------------
// geometry
// ---------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Color {
    pub const TRANSPARENT: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const RED: Self = Self { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const GREEN: Self = Self { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
    pub const BLUE: Self = Self { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Extent3d {
    pub width: u32,
    pub height: u32,
    pub depth_or_array_layers: u32,
}

impl Default for Extent3d {
    fn default() -> Self {
        Self { width: 1, height: 1, depth_or_array_layers: 1 }
    }
}

impl Extent3d {
    pub fn physical_size(&self, format: TextureFormat) -> Self {
        let (block_width, block_height) = format.block_dimensions();
        Self {
            width: self.width.div_ceil(block_width) * block_width,
            height: self.height.div_ceil(block_height) * block_height,
            depth_or_array_layers: self.depth_or_array_layers,
        }
    }
    pub fn max_mips(&self, dimension: TextureDimension) -> u32 {
        match dimension {
            TextureDimension::D1 => 1,
            TextureDimension::D2 => {
                let max_dim = self.width.max(self.height);
                32 - max_dim.leading_zeros()
            }
            TextureDimension::D3 => {
                let max_dim = self.width.max(self.height.max(self.depth_or_array_layers));
                32 - max_dim.leading_zeros()
            }
        }
    }
    pub fn mip_level_size(&self, level: u32, dimension: TextureDimension) -> Self {
        Self {
            width: u32::max(1, self.width >> level),
            height: match dimension {
                TextureDimension::D1 => 1,
                _ => u32::max(1, self.height >> level),
            },
            depth_or_array_layers: match dimension {
                TextureDimension::D1 | TextureDimension::D2 => self.depth_or_array_layers,
                TextureDimension::D3 => u32::max(1, self.depth_or_array_layers >> level),
            },
        }
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct Origin3d {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Origin3d {
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct Origin2d {
    pub x: u32,
    pub y: u32,
}

// ---------------------------------------------------------------------
// texture-related enums
// ---------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum TextureDimension {
    D1,
    #[default]
    D2,
    D3,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum TextureViewDimension {
    D1,
    #[default]
    D2,
    D2Array,
    Cube,
    CubeArray,
    D3,
}

impl TextureViewDimension {
    pub fn compatible_texture_dimension(self) -> TextureDimension {
        match self {
            Self::D1 => TextureDimension::D1,
            Self::D2 | Self::D2Array | Self::Cube | Self::CubeArray => TextureDimension::D2,
            Self::D3 => TextureDimension::D3,
        }
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum TextureAspect {
    #[default]
    All,
    StencilOnly,
    DepthOnly,
    Plane0,
    Plane1,
    Plane2,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum AstcBlock {
    B4x4,
    B5x4,
    B5x5,
    B6x5,
    B6x6,
    B8x5,
    B8x6,
    B8x8,
    B10x5,
    B10x6,
    B10x8,
    B10x10,
    B12x10,
    B12x12,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum AstcChannel {
    Unorm,
    UnormSrgb,
    Hdr,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    R8Unorm,
    R8Snorm,
    R8Uint,
    R8Sint,
    R16Uint,
    R16Sint,
    R16Unorm,
    R16Snorm,
    R16Float,
    Rg8Unorm,
    Rg8Snorm,
    Rg8Uint,
    Rg8Sint,
    R32Uint,
    R32Sint,
    R32Float,
    Rg16Uint,
    Rg16Sint,
    Rg16Unorm,
    Rg16Snorm,
    Rg16Float,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Rgba8Snorm,
    Rgba8Uint,
    Rgba8Sint,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgb9e5Ufloat,
    Rgb10a2Uint,
    Rgb10a2Unorm,
    Rg11b10Ufloat,
    Rg32Uint,
    Rg32Sint,
    Rg32Float,
    Rgba16Uint,
    Rgba16Sint,
    Rgba16Unorm,
    Rgba16Snorm,
    Rgba16Float,
    Rgba32Uint,
    Rgba32Sint,
    Rgba32Float,
    Stencil8,
    Depth16Unorm,
    Depth24Plus,
    Depth24PlusStencil8,
    Depth32Float,
    Depth32FloatStencil8,
    Bc1RgbaUnorm,
    Bc1RgbaUnormSrgb,
    Bc2RgbaUnorm,
    Bc2RgbaUnormSrgb,
    Bc3RgbaUnorm,
    Bc3RgbaUnormSrgb,
    Bc4RUnorm,
    Bc4RSnorm,
    Bc5RgUnorm,
    Bc5RgSnorm,
    Bc6hRgbUfloat,
    Bc6hRgbFloat,
    Bc7RgbaUnorm,
    Bc7RgbaUnormSrgb,
    Etc2Rgb8Unorm,
    Etc2Rgb8UnormSrgb,
    Etc2Rgb8A1Unorm,
    Etc2Rgb8A1UnormSrgb,
    Etc2Rgba8Unorm,
    Etc2Rgba8UnormSrgb,
    EacR11Unorm,
    EacR11Snorm,
    EacRg11Unorm,
    EacRg11Snorm,
    Astc { block: AstcBlock, channel: AstcChannel },
}

impl TextureFormat {
    pub fn add_srgb_suffix(self) -> Self {
        match self {
            Self::Rgba8Unorm => Self::Rgba8UnormSrgb,
            Self::Bgra8Unorm => Self::Bgra8UnormSrgb,
            Self::Bc1RgbaUnorm => Self::Bc1RgbaUnormSrgb,
            Self::Bc2RgbaUnorm => Self::Bc2RgbaUnormSrgb,
            Self::Bc3RgbaUnorm => Self::Bc3RgbaUnormSrgb,
            Self::Bc7RgbaUnorm => Self::Bc7RgbaUnormSrgb,
            Self::Etc2Rgb8Unorm => Self::Etc2Rgb8UnormSrgb,
            Self::Etc2Rgb8A1Unorm => Self::Etc2Rgb8A1UnormSrgb,
            Self::Etc2Rgba8Unorm => Self::Etc2Rgba8UnormSrgb,
            Self::Astc { block, channel: AstcChannel::Unorm } => {
                Self::Astc { block, channel: AstcChannel::UnormSrgb }
            }
            _ => self,
        }
    }

    pub fn remove_srgb_suffix(self) -> Self {
        match self {
            Self::Rgba8UnormSrgb => Self::Rgba8Unorm,
            Self::Bgra8UnormSrgb => Self::Bgra8Unorm,
            Self::Bc1RgbaUnormSrgb => Self::Bc1RgbaUnorm,
            Self::Bc2RgbaUnormSrgb => Self::Bc2RgbaUnorm,
            Self::Bc3RgbaUnormSrgb => Self::Bc3RgbaUnorm,
            Self::Bc7RgbaUnormSrgb => Self::Bc7RgbaUnorm,
            Self::Etc2Rgb8UnormSrgb => Self::Etc2Rgb8Unorm,
            Self::Etc2Rgb8A1UnormSrgb => Self::Etc2Rgb8A1Unorm,
            Self::Etc2Rgba8UnormSrgb => Self::Etc2Rgba8Unorm,
            Self::Astc { block, channel: AstcChannel::UnormSrgb } => {
                Self::Astc { block, channel: AstcChannel::Unorm }
            }
            _ => self,
        }
    }

    pub fn is_srgb(&self) -> bool {
        *self != self.remove_srgb_suffix()
    }

    pub fn block_dimensions(&self) -> (u32, u32) {
        match self {
            Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc4RUnorm
            | Self::Bc4RSnorm
            | Self::Bc5RgUnorm
            | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat
            | Self::Bc6hRgbFloat
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb
            | Self::Etc2Rgb8Unorm
            | Self::Etc2Rgb8UnormSrgb
            | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb
            | Self::EacR11Unorm
            | Self::EacR11Snorm
            | Self::EacRg11Unorm
            | Self::EacRg11Snorm => (4, 4),
            Self::Astc { block, .. } => match block {
                AstcBlock::B4x4 => (4, 4),
                AstcBlock::B5x4 => (5, 4),
                AstcBlock::B5x5 => (5, 5),
                AstcBlock::B6x5 => (6, 5),
                AstcBlock::B6x6 => (6, 6),
                AstcBlock::B8x5 => (8, 5),
                AstcBlock::B8x6 => (8, 6),
                AstcBlock::B8x8 => (8, 8),
                AstcBlock::B10x5 => (10, 5),
                AstcBlock::B10x6 => (10, 6),
                AstcBlock::B10x8 => (10, 8),
                AstcBlock::B10x10 => (10, 10),
                AstcBlock::B12x10 => (12, 10),
                AstcBlock::B12x12 => (12, 12),
            },
            _ => (1, 1),
        }
    }

    pub fn block_copy_size(&self, aspect: Option<TextureAspect>) -> Option<u32> {
        let _ = aspect;
        Some(match self {
            Self::R8Unorm | Self::R8Snorm | Self::R8Uint | Self::R8Sint | Self::Stencil8 => 1,
            Self::R16Uint | Self::R16Sint | Self::R16Unorm | Self::R16Snorm | Self::R16Float
            | Self::Rg8Unorm | Self::Rg8Snorm | Self::Rg8Uint | Self::Rg8Sint
            | Self::Depth16Unorm => 2,
            Self::R32Uint | Self::R32Sint | Self::R32Float | Self::Rg16Uint | Self::Rg16Sint
            | Self::Rg16Unorm | Self::Rg16Snorm | Self::Rg16Float | Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb | Self::Rgba8Snorm | Self::Rgba8Uint | Self::Rgba8Sint
            | Self::Bgra8Unorm | Self::Bgra8UnormSrgb | Self::Rgb9e5Ufloat | Self::Rgb10a2Uint
            | Self::Rgb10a2Unorm | Self::Rg11b10Ufloat | Self::Depth32Float => 4,
            Self::Rg32Uint | Self::Rg32Sint | Self::Rg32Float | Self::Rgba16Uint
            | Self::Rgba16Sint | Self::Rgba16Unorm | Self::Rgba16Snorm | Self::Rgba16Float => 8,
            Self::Rgba32Uint | Self::Rgba32Sint | Self::Rgba32Float => 16,
            Self::Bc1RgbaUnorm | Self::Bc1RgbaUnormSrgb | Self::Bc4RUnorm | Self::Bc4RSnorm
            | Self::Etc2Rgb8Unorm | Self::Etc2Rgb8UnormSrgb | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb | Self::EacR11Unorm | Self::EacR11Snorm => 8,
            Self::Bc2RgbaUnorm | Self::Bc2RgbaUnormSrgb | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb | Self::Bc5RgUnorm | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat | Self::Bc6hRgbFloat | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb | Self::Etc2Rgba8Unorm | Self::Etc2Rgba8UnormSrgb
            | Self::EacRg11Unorm | Self::EacRg11Snorm | Self::Astc { .. } => 16,
            Self::Depth24Plus | Self::Depth24PlusStencil8 | Self::Depth32FloatStencil8 => {
                return None
            }
        })
    }
}

// ---------------------------------------------------------------------
// samplers
// ---------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum AddressMode {
    #[default]
    ClampToEdge,
    Repeat,
    MirrorRepeat,
    ClampToBorder,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum FilterMode {
    #[default]
    Nearest,
    Linear,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum MipmapFilterMode {
    #[default]
    Nearest,
    Linear,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CompareFunction {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

impl CompareFunction {
    pub fn needs_ref_value(self) -> bool {
        !matches!(self, Self::Never | Self::Always)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum SamplerBorderColor {
    TransparentBlack,
    OpaqueBlack,
    OpaqueWhite,
    Zero,
}

#[derive(Debug, Clone)]
pub struct SamplerDescriptor<'a> {
    pub label: Label<'a>,
    pub address_mode_u: AddressMode,
    pub address_mode_v: AddressMode,
    pub address_mode_w: AddressMode,
    pub mag_filter: FilterMode,
    pub min_filter: FilterMode,
    pub mipmap_filter: MipmapFilterMode,
    pub lod_min_clamp: f32,
    pub lod_max_clamp: f32,
    pub compare: Option<CompareFunction>,
    pub anisotropy_clamp: u16,
    pub border_color: Option<SamplerBorderColor>,
}

impl Default for SamplerDescriptor<'_> {
    fn default() -> Self {
        Self {
            label: None,
            address_mode_u: AddressMode::default(),
            address_mode_v: AddressMode::default(),
            address_mode_w: AddressMode::default(),
            mag_filter: FilterMode::default(),
            min_filter: FilterMode::default(),
            mipmap_filter: MipmapFilterMode::default(),
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        }
    }
}

// ---------------------------------------------------------------------
// buffers and textures
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct BufferDescriptor<'a> {
    pub label: Label<'a>,
    pub size: BufferAddress,
    pub usage: BufferUsages,
    pub mapped_at_creation: bool,
}

#[derive(Debug, Clone)]
pub struct TextureDescriptor<'a> {
    pub label: Label<'a>,
    pub size: Extent3d,
    pub mip_level_count: u32,
    pub sample_count: u32,
    pub dimension: TextureDimension,
    pub format: TextureFormat,
    pub usage: TextureUsages,
    pub view_formats: &'a [TextureFormat],
}

impl TextureDescriptor<'_> {
    pub fn mip_level_size(&self, level: u32) -> Option<Extent3d> {
        if level >= self.mip_level_count {
            return None;
        }
        Some(self.size.mip_level_size(level, self.dimension))
    }
    pub fn array_layer_count(&self) -> u32 {
        match self.dimension {
            TextureDimension::D1 | TextureDimension::D3 => 1,
            TextureDimension::D2 => self.size.depth_or_array_layers,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextureViewDescriptor<'a> {
    pub label: Label<'a>,
    pub format: Option<TextureFormat>,
    pub dimension: Option<TextureViewDimension>,
    pub usage: Option<TextureUsages>,
    pub aspect: TextureAspect,
    pub base_mip_level: u32,
    pub mip_level_count: Option<u32>,
    pub base_array_layer: u32,
    pub array_layer_count: Option<u32>,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum MapMode {
    #[default]
    Read,
    Write,
}

#[derive(Debug, Clone, Default)]
pub struct TexelCopyBufferLayout {
    pub offset: BufferAddress,
    pub bytes_per_row: Option<u32>,
    pub rows_per_image: Option<u32>,
}

#[derive(Clone)]
pub struct TexelCopyTextureInfo<'a> {
    pub texture: &'a crate::Texture,
    pub mip_level: u32,
    pub origin: Origin3d,
    pub aspect: TextureAspect,
}

#[derive(Clone)]
pub struct TexelCopyBufferInfo<'a> {
    pub buffer: &'a crate::Buffer,
    pub layout: TexelCopyBufferLayout,
}

#[derive(Debug, Clone, Default)]
pub struct ImageSubresourceRange {
    pub aspect: TextureAspect,
    pub base_mip_level: u32,
    pub mip_level_count: Option<u32>,
    pub base_array_layer: u32,
    pub array_layer_count: Option<u32>,
}

// ---------------------------------------------------------------------
// binding model
// ---------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum BufferBindingType {
    #[default]
    Uniform,
    Storage {
        read_only: bool,
    },
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum SamplerBindingType {
    #[default]
    Filtering,
    NonFiltering,
    Comparison,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextureSampleType {
    Float { filterable: bool },
    Depth,
    Sint,
    Uint,
}
impl Default for TextureSampleType {
    fn default() -> Self {
        Self::Float { filterable: true }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum StorageTextureAccess {
    WriteOnly,
    ReadOnly,
    ReadWrite,
    Atomic,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum BindingType {
    Buffer {
        ty: BufferBindingType,
        has_dynamic_offset: bool,
        min_binding_size: Option<BufferSize>,
    },
    Sampler(SamplerBindingType),
    Texture {
        sample_type: TextureSampleType,
        view_dimension: TextureViewDimension,
        multisampled: bool,
    },
    StorageTexture {
        access: StorageTextureAccess,
        format: TextureFormat,
        view_dimension: TextureViewDimension,
    },
    AccelerationStructure {
        vertex_return: bool,
    },
    ExternalTexture,
}

#[derive(Debug, Copy, Clone)]
pub struct BindGroupLayoutEntry {
    pub binding: u32,
    pub visibility: ShaderStages,
    pub ty: BindingType,
    pub count: Option<NonZeroU32>,
}

#[derive(Debug, Clone)]
pub struct BindGroupLayoutDescriptor<'a> {
    pub label: Label<'a>,
    pub entries: &'a [BindGroupLayoutEntry],
}

#[derive(Clone)]
pub struct BufferBinding<'a> {
    pub buffer: &'a crate::Buffer,
    pub offset: BufferAddress,
    pub size: Option<BufferSize>,
}

#[derive(Clone)]
pub enum BindingResource<'a> {
    Buffer(BufferBinding<'a>),
    BufferArray(&'a [BufferBinding<'a>]),
    Sampler(&'a crate::Sampler),
    SamplerArray(&'a [&'a crate::Sampler]),
    TextureView(&'a crate::TextureView),
    TextureViewArray(&'a [&'a crate::TextureView]),
}

#[derive(Clone)]
pub struct BindGroupEntry<'a> {
    pub binding: u32,
    pub resource: BindingResource<'a>,
}

#[derive(Clone)]
pub struct BindGroupDescriptor<'a> {
    pub label: Label<'a>,
    pub layout: &'a crate::BindGroupLayout,
    pub entries: &'a [BindGroupEntry<'a>],
}

#[derive(Clone)]
pub struct PipelineLayoutDescriptor<'a> {
    pub label: Label<'a>,
    pub bind_group_layouts: &'a [Option<&'a crate::BindGroupLayout>],
    pub immediate_size: u32,
}

impl Default for PipelineLayoutDescriptor<'_> {
    fn default() -> Self {
        Self { label: None, bind_group_layouts: &[], immediate_size: 0 }
    }
}

// ---------------------------------------------------------------------
// shaders and pipelines
// ---------------------------------------------------------------------

pub enum ShaderSource<'a> {
    Wgsl(Cow<'a, str>),
}

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

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct VertexAttribute {
    pub format: VertexFormat,
    pub offset: BufferAddress,
    pub shader_location: ShaderLocation,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum VertexStepMode {
    #[default]
    Vertex,
    Instance,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VertexFormat {
    Uint8,
    Uint8x2,
    Uint8x4,
    Sint8,
    Sint8x2,
    Sint8x4,
    Unorm8,
    Unorm8x2,
    Unorm8x4,
    Snorm8,
    Snorm8x2,
    Snorm8x4,
    Uint16,
    Uint16x2,
    Uint16x4,
    Sint16,
    Sint16x2,
    Sint16x4,
    Unorm16,
    Unorm16x2,
    Unorm16x4,
    Snorm16,
    Snorm16x2,
    Snorm16x4,
    Float16,
    Float16x2,
    Float16x4,
    Float32,
    Float32x2,
    Float32x3,
    Float32x4,
    Uint32,
    Uint32x2,
    Uint32x3,
    Uint32x4,
    Sint32,
    Sint32x2,
    Sint32x3,
    Sint32x4,
    Float64,
    Float64x2,
    Float64x3,
    Float64x4,
    Unorm10_10_10_2,
    Unorm8x4Bgra,
}

impl VertexFormat {
    pub const fn size(&self) -> u64 {
        match self {
            Self::Uint8 | Self::Sint8 | Self::Unorm8 | Self::Snorm8 => 1,
            Self::Uint8x2 | Self::Sint8x2 | Self::Unorm8x2 | Self::Snorm8x2 | Self::Uint16
            | Self::Sint16 | Self::Unorm16 | Self::Snorm16 | Self::Float16 => 2,
            Self::Uint8x4 | Self::Sint8x4 | Self::Unorm8x4 | Self::Snorm8x4 | Self::Uint16x2
            | Self::Sint16x2 | Self::Unorm16x2 | Self::Snorm16x2 | Self::Float16x2
            | Self::Float32 | Self::Uint32 | Self::Sint32 | Self::Unorm10_10_10_2
            | Self::Unorm8x4Bgra => 4,
            Self::Uint16x4 | Self::Sint16x4 | Self::Unorm16x4 | Self::Snorm16x4
            | Self::Float16x4 | Self::Float32x2 | Self::Uint32x2 | Self::Sint32x2
            | Self::Float64 => 8,
            Self::Float32x3 | Self::Uint32x3 | Self::Sint32x3 => 12,
            Self::Float32x4 | Self::Uint32x4 | Self::Sint32x4 | Self::Float64x2 => 16,
            Self::Float64x3 => 24,
            Self::Float64x4 => 32,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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
    pub buffers: &'a [Option<VertexBufferLayout<'a>>],
}

#[derive(Clone)]
pub struct FragmentState<'a> {
    pub module: &'a crate::ShaderModule,
    pub entry_point: Option<&'a str>,
    pub compilation_options: PipelineCompilationOptions<'a>,
    pub targets: &'a [Option<ColorTargetState>],
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ColorTargetState {
    pub format: TextureFormat,
    pub blend: Option<BlendState>,
    pub write_mask: ColorWrites,
}

impl From<TextureFormat> for ColorTargetState {
    fn from(format: TextureFormat) -> Self {
        Self { format, blend: None, write_mask: ColorWrites::ALL }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BlendState {
    pub color: BlendComponent,
    pub alpha: BlendComponent,
}

impl BlendState {
    pub const REPLACE: Self = Self {
        color: BlendComponent::REPLACE,
        alpha: BlendComponent::REPLACE,
    };
    pub const ALPHA_BLENDING: Self = Self {
        color: BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::OVER,
    };
    pub const PREMULTIPLIED_ALPHA_BLENDING: Self = Self {
        color: BlendComponent::OVER,
        alpha: BlendComponent::OVER,
    };
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BlendComponent {
    pub src_factor: BlendFactor,
    pub dst_factor: BlendFactor,
    pub operation: BlendOperation,
}

impl BlendComponent {
    pub const REPLACE: Self = Self {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::Zero,
        operation: BlendOperation::Add,
    };
    pub const OVER: Self = Self {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::OneMinusSrcAlpha,
        operation: BlendOperation::Add,
    };
}

impl Default for BlendComponent {
    fn default() -> Self {
        Self::REPLACE
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum BlendFactor {
    Zero,
    One,
    Src,
    OneMinusSrc,
    SrcAlpha,
    OneMinusSrcAlpha,
    Dst,
    OneMinusDst,
    DstAlpha,
    OneMinusDstAlpha,
    SrcAlphaSaturated,
    Constant,
    OneMinusConstant,
    Src1,
    OneMinusSrc1,
    Src1Alpha,
    OneMinusSrc1Alpha,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum BlendOperation {
    #[default]
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum PrimitiveTopology {
    PointList,
    LineList,
    LineStrip,
    #[default]
    TriangleList,
    TriangleStrip,
}

impl PrimitiveTopology {
    pub fn is_strip(&self) -> bool {
        matches!(self, Self::LineStrip | Self::TriangleStrip)
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum FrontFace {
    #[default]
    Ccw,
    Cw,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Face {
    Front,
    Back,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum PolygonMode {
    #[default]
    Fill,
    Line,
    Point,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum IndexFormat {
    Uint16,
    #[default]
    Uint32,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct PrimitiveState {
    pub topology: PrimitiveTopology,
    pub strip_index_format: Option<IndexFormat>,
    pub front_face: FrontFace,
    pub cull_mode: Option<Face>,
    pub unclipped_depth: bool,
    pub polygon_mode: PolygonMode,
    pub conservative: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum StencilOperation {
    Keep,
    Zero,
    Replace,
    Invert,
    IncrementClamp,
    DecrementClamp,
    IncrementWrap,
    DecrementWrap,
}

impl Default for StencilOperation {
    fn default() -> Self {
        Self::Keep
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StencilFaceState {
    pub compare: CompareFunction,
    pub fail_op: StencilOperation,
    pub depth_fail_op: StencilOperation,
    pub pass_op: StencilOperation,
}

impl StencilFaceState {
    pub const IGNORE: Self = Self {
        compare: CompareFunction::Always,
        fail_op: StencilOperation::Keep,
        depth_fail_op: StencilOperation::Keep,
        pass_op: StencilOperation::Keep,
    };
}

impl Default for StencilFaceState {
    fn default() -> Self {
        Self::IGNORE
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct StencilState {
    pub front: StencilFaceState,
    pub back: StencilFaceState,
    pub read_mask: u32,
    pub write_mask: u32,
}

impl StencilState {
    pub fn is_enabled(&self) -> bool {
        (self.front != StencilFaceState::IGNORE || self.back != StencilFaceState::IGNORE)
            && (self.read_mask != 0 || self.write_mask != 0)
    }
    pub fn is_read_only(&self, cull_mode: Option<Face>) -> bool {
        let _ = cull_mode;
        self.write_mask == 0
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct DepthBiasState {
    pub constant: i32,
    pub slope_scale: f32,
    pub clamp: f32,
}

impl DepthBiasState {
    pub fn is_enabled(&self) -> bool {
        self.constant != 0 || self.slope_scale != 0.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DepthStencilState {
    pub format: TextureFormat,
    pub depth_write_enabled: Option<bool>,
    pub depth_compare: Option<CompareFunction>,
    pub stencil: StencilState,
    pub bias: DepthBiasState,
}

impl DepthStencilState {
    /// Construct `DepthStencilState` for a stencil operation with no depth
    /// operation.
    pub fn stencil(format: TextureFormat, stencil: StencilState) -> DepthStencilState {
        DepthStencilState {
            format,
            depth_write_enabled: None,
            depth_compare: None,
            stencil,
            bias: DepthBiasState::default(),
        }
    }
    pub fn is_depth_enabled(&self) -> bool {
        self.depth_compare.unwrap_or(CompareFunction::Always) != CompareFunction::Always
            || self.depth_write_enabled.unwrap_or(false)
    }
    pub fn is_depth_read_only(&self) -> bool {
        !self.depth_write_enabled.unwrap_or(false)
    }
    pub fn is_stencil_read_only(&self, cull_mode: Option<Face>) -> bool {
        self.stencil.is_read_only(cull_mode)
    }
    pub fn is_read_only(&self, cull_mode: Option<Face>) -> bool {
        self.is_depth_read_only() && self.is_stencil_read_only(cull_mode)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct MultisampleState {
    pub count: u32,
    pub mask: u64,
    pub alpha_to_coverage_enabled: bool,
}

impl Default for MultisampleState {
    fn default() -> Self {
        Self { count: 1, mask: !0, alpha_to_coverage_enabled: false }
    }
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

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub enum LoadOp<V> {
    Clear(V),
    #[default]
    Load,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum StoreOp {
    #[default]
    Store,
    Discard,
}

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Operations<V> {
    pub load: LoadOp<V>,
    pub store: StoreOp,
}

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
    pub multiview_mask: Option<std::num::NonZeroU32>,
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
pub struct CommandEncoderDescriptor<'a> {
    pub label: Label<'a>,
}

#[derive(Debug, Clone, Default)]
pub struct CommandBufferDescriptor<'a> {
    pub label: Label<'a>,
}

#[derive(Debug, Clone, Default)]
pub struct RenderBundleDescriptor<'a> {
    pub label: Label<'a>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct RenderBundleDepthStencil {
    pub format: TextureFormat,
    pub depth_read_only: bool,
    pub stencil_read_only: bool,
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
// queries
// ---------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum QueryType {
    Occlusion,
    Timestamp,
    PipelineStatistics(PipelineStatisticsTypes),
}

#[derive(Debug, Clone)]
pub struct QuerySetDescriptor<'a> {
    pub label: Label<'a>,
    pub ty: QueryType,
    pub count: u32,
}

// ---------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum PresentMode {
    AutoVsync,
    AutoNoVsync,
    #[default]
    Fifo,
    FifoRelaxed,
    Immediate,
    Mailbox,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum CompositeAlphaMode {
    #[default]
    Auto,
    Opaque,
    PreMultiplied,
    PostMultiplied,
    Inherit,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Hash)]
pub enum SurfaceColorSpace {
    #[default]
    Auto,
    Srgb,
    DisplayP3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceConfiguration {
    pub usage: TextureUsages,
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub present_mode: PresentMode,
    pub desired_maximum_frame_latency: u32,
    pub alpha_mode: CompositeAlphaMode,
    pub view_formats: Vec<TextureFormat>,
    pub color_space: SurfaceColorSpace,
}

#[derive(Debug, Clone, Default)]
pub struct SurfaceCapabilities {
    pub formats: Vec<TextureFormat>,
    pub present_modes: Vec<PresentMode>,
    pub alpha_modes: Vec<CompositeAlphaMode>,
    pub usages: TextureUsages,
}

// ---------------------------------------------------------------------
// errors
// ---------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ErrorFilter {
    Validation,
    OutOfMemory,
    Internal,
}

#[derive(Debug, Clone)]
pub enum Error {
    OutOfMemory { source: ErrorSource },
    Validation { source: ErrorSource, description: String },
    Internal { source: ErrorSource, description: String },
}

pub type ErrorSource = std::sync::Arc<dyn std::error::Error + Send + Sync + 'static>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::OutOfMemory { .. } => write!(f, "Out of Memory"),
            Error::Validation { description, .. } => write!(f, "Validation Error: {description}"),
            Error::Internal { description, .. } => write!(f, "Internal Error: {description}"),
        }
    }
}

impl std::error::Error for Error {}
