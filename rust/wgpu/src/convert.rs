//! Conversions from the public API types to the C `webgpu.h` types.

use remote_wgpu_sys as sys;

use crate::types::*;

pub(crate) fn sv(label: Option<&str>) -> sys::WGPUStringView {
    match label {
        Some(s) => sv_str(s),
        None => sys::WGPUStringView { data: std::ptr::null(), length: 0 },
    }
}

pub(crate) fn sv_str(s: &str) -> sys::WGPUStringView {
    sys::WGPUStringView { data: s.as_ptr() as *const _, length: s.len() }
}

pub(crate) fn from_sv(view: sys::WGPUStringView) -> String {
    if view.data.is_null() || view.length == 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(view.data as *const u8, view.length) };
    String::from_utf8_lossy(bytes).into_owned()
}

pub(crate) fn bool32(value: bool) -> sys::WGPUBool {
    if value { 1 } else { 0 }
}

pub(crate) fn map_texture_format(format: TextureFormat) -> sys::WGPUTextureFormat {
    use TextureFormat as F;
    match format {
        F::R64Uint | F::NV12 | F::P010 => {
            panic!("texture format {format:?} is not available over remote WebGPU")
        }
        F::R8Unorm => sys::WGPUTextureFormat_R8Unorm,
        F::R8Snorm => sys::WGPUTextureFormat_R8Snorm,
        F::R8Uint => sys::WGPUTextureFormat_R8Uint,
        F::R8Sint => sys::WGPUTextureFormat_R8Sint,
        F::R16Uint => sys::WGPUTextureFormat_R16Uint,
        F::R16Sint => sys::WGPUTextureFormat_R16Sint,
        F::R16Float => sys::WGPUTextureFormat_R16Float,
        F::Rg8Unorm => sys::WGPUTextureFormat_RG8Unorm,
        F::Rg8Snorm => sys::WGPUTextureFormat_RG8Snorm,
        F::Rg8Uint => sys::WGPUTextureFormat_RG8Uint,
        F::Rg8Sint => sys::WGPUTextureFormat_RG8Sint,
        F::R32Uint => sys::WGPUTextureFormat_R32Uint,
        F::R32Sint => sys::WGPUTextureFormat_R32Sint,
        F::R32Float => sys::WGPUTextureFormat_R32Float,
        F::Rg16Uint => sys::WGPUTextureFormat_RG16Uint,
        F::Rg16Sint => sys::WGPUTextureFormat_RG16Sint,
        F::Rg16Float => sys::WGPUTextureFormat_RG16Float,
        F::Rgba8Unorm => sys::WGPUTextureFormat_RGBA8Unorm,
        F::Rgba8UnormSrgb => sys::WGPUTextureFormat_RGBA8UnormSrgb,
        F::Rgba8Snorm => sys::WGPUTextureFormat_RGBA8Snorm,
        F::Rgba8Uint => sys::WGPUTextureFormat_RGBA8Uint,
        F::Rgba8Sint => sys::WGPUTextureFormat_RGBA8Sint,
        F::Bgra8Unorm => sys::WGPUTextureFormat_BGRA8Unorm,
        F::Bgra8UnormSrgb => sys::WGPUTextureFormat_BGRA8UnormSrgb,
        F::Rgb9e5Ufloat => sys::WGPUTextureFormat_RGB9E5Ufloat,
        F::Rgb10a2Uint => sys::WGPUTextureFormat_RGB10A2Uint,
        F::Rgb10a2Unorm => sys::WGPUTextureFormat_RGB10A2Unorm,
        F::Rg11b10Ufloat => sys::WGPUTextureFormat_RG11B10Ufloat,
        F::Rg32Uint => sys::WGPUTextureFormat_RG32Uint,
        F::Rg32Sint => sys::WGPUTextureFormat_RG32Sint,
        F::Rg32Float => sys::WGPUTextureFormat_RG32Float,
        F::Rgba16Uint => sys::WGPUTextureFormat_RGBA16Uint,
        F::Rgba16Sint => sys::WGPUTextureFormat_RGBA16Sint,
        F::Rgba16Float => sys::WGPUTextureFormat_RGBA16Float,
        F::Rgba32Uint => sys::WGPUTextureFormat_RGBA32Uint,
        F::Rgba32Sint => sys::WGPUTextureFormat_RGBA32Sint,
        F::Rgba32Float => sys::WGPUTextureFormat_RGBA32Float,
        F::Stencil8 => sys::WGPUTextureFormat_Stencil8,
        F::Depth16Unorm => sys::WGPUTextureFormat_Depth16Unorm,
        F::Depth24Plus => sys::WGPUTextureFormat_Depth24Plus,
        F::Depth24PlusStencil8 => sys::WGPUTextureFormat_Depth24PlusStencil8,
        F::Depth32Float => sys::WGPUTextureFormat_Depth32Float,
        F::Depth32FloatStencil8 => sys::WGPUTextureFormat_Depth32FloatStencil8,
        F::Bc1RgbaUnorm => sys::WGPUTextureFormat_BC1RGBAUnorm,
        F::Bc1RgbaUnormSrgb => sys::WGPUTextureFormat_BC1RGBAUnormSrgb,
        F::Bc2RgbaUnorm => sys::WGPUTextureFormat_BC2RGBAUnorm,
        F::Bc2RgbaUnormSrgb => sys::WGPUTextureFormat_BC2RGBAUnormSrgb,
        F::Bc3RgbaUnorm => sys::WGPUTextureFormat_BC3RGBAUnorm,
        F::Bc3RgbaUnormSrgb => sys::WGPUTextureFormat_BC3RGBAUnormSrgb,
        F::Bc4RUnorm => sys::WGPUTextureFormat_BC4RUnorm,
        F::Bc4RSnorm => sys::WGPUTextureFormat_BC4RSnorm,
        F::Bc5RgUnorm => sys::WGPUTextureFormat_BC5RGUnorm,
        F::Bc5RgSnorm => sys::WGPUTextureFormat_BC5RGSnorm,
        F::Bc6hRgbUfloat => sys::WGPUTextureFormat_BC6HRGBUfloat,
        F::Bc6hRgbFloat => sys::WGPUTextureFormat_BC6HRGBFloat,
        F::Bc7RgbaUnorm => sys::WGPUTextureFormat_BC7RGBAUnorm,
        F::Bc7RgbaUnormSrgb => sys::WGPUTextureFormat_BC7RGBAUnormSrgb,
        F::Etc2Rgb8Unorm => sys::WGPUTextureFormat_ETC2RGB8Unorm,
        F::Etc2Rgb8UnormSrgb => sys::WGPUTextureFormat_ETC2RGB8UnormSrgb,
        F::Etc2Rgb8A1Unorm => sys::WGPUTextureFormat_ETC2RGB8A1Unorm,
        F::Etc2Rgb8A1UnormSrgb => sys::WGPUTextureFormat_ETC2RGB8A1UnormSrgb,
        F::Etc2Rgba8Unorm => sys::WGPUTextureFormat_ETC2RGBA8Unorm,
        F::Etc2Rgba8UnormSrgb => sys::WGPUTextureFormat_ETC2RGBA8UnormSrgb,
        F::EacR11Unorm => sys::WGPUTextureFormat_EACR11Unorm,
        F::EacR11Snorm => sys::WGPUTextureFormat_EACR11Snorm,
        F::EacRg11Unorm => sys::WGPUTextureFormat_EACRG11Unorm,
        F::EacRg11Snorm => sys::WGPUTextureFormat_EACRG11Snorm,
        F::R16Unorm | F::R16Snorm | F::Rg16Unorm | F::Rg16Snorm | F::Rgba16Unorm
        | F::Rgba16Snorm => {
            panic!("{format:?} is a native-only format not available in WebGPU")
        }
        F::Astc { block, channel } => {
            use AstcBlock as B;
            use AstcChannel as C;
            match (block, channel) {
                (B::B4x4, C::Unorm) => sys::WGPUTextureFormat_ASTC4x4Unorm,
                (B::B4x4, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC4x4UnormSrgb,
                (B::B5x4, C::Unorm) => sys::WGPUTextureFormat_ASTC5x4Unorm,
                (B::B5x4, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC5x4UnormSrgb,
                (B::B5x5, C::Unorm) => sys::WGPUTextureFormat_ASTC5x5Unorm,
                (B::B5x5, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC5x5UnormSrgb,
                (B::B6x5, C::Unorm) => sys::WGPUTextureFormat_ASTC6x5Unorm,
                (B::B6x5, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC6x5UnormSrgb,
                (B::B6x6, C::Unorm) => sys::WGPUTextureFormat_ASTC6x6Unorm,
                (B::B6x6, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC6x6UnormSrgb,
                (B::B8x5, C::Unorm) => sys::WGPUTextureFormat_ASTC8x5Unorm,
                (B::B8x5, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC8x5UnormSrgb,
                (B::B8x6, C::Unorm) => sys::WGPUTextureFormat_ASTC8x6Unorm,
                (B::B8x6, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC8x6UnormSrgb,
                (B::B8x8, C::Unorm) => sys::WGPUTextureFormat_ASTC8x8Unorm,
                (B::B8x8, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC8x8UnormSrgb,
                (B::B10x5, C::Unorm) => sys::WGPUTextureFormat_ASTC10x5Unorm,
                (B::B10x5, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC10x5UnormSrgb,
                (B::B10x6, C::Unorm) => sys::WGPUTextureFormat_ASTC10x6Unorm,
                (B::B10x6, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC10x6UnormSrgb,
                (B::B10x8, C::Unorm) => sys::WGPUTextureFormat_ASTC10x8Unorm,
                (B::B10x8, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC10x8UnormSrgb,
                (B::B10x10, C::Unorm) => sys::WGPUTextureFormat_ASTC10x10Unorm,
                (B::B10x10, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC10x10UnormSrgb,
                (B::B12x10, C::Unorm) => sys::WGPUTextureFormat_ASTC12x10Unorm,
                (B::B12x10, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC12x10UnormSrgb,
                (B::B12x12, C::Unorm) => sys::WGPUTextureFormat_ASTC12x12Unorm,
                (B::B12x12, C::UnormSrgb) => sys::WGPUTextureFormat_ASTC12x12UnormSrgb,
                (_, C::Hdr) => panic!("ASTC HDR is not available in WebGPU"),
            }
        }
    }
}

pub(crate) fn unmap_texture_format(format: sys::WGPUTextureFormat) -> Option<TextureFormat> {
    use TextureFormat as F;
    Some(match format {
        x if x == sys::WGPUTextureFormat_R8Unorm => F::R8Unorm,
        x if x == sys::WGPUTextureFormat_R8Snorm => F::R8Snorm,
        x if x == sys::WGPUTextureFormat_R8Uint => F::R8Uint,
        x if x == sys::WGPUTextureFormat_R8Sint => F::R8Sint,
        x if x == sys::WGPUTextureFormat_R16Uint => F::R16Uint,
        x if x == sys::WGPUTextureFormat_R16Sint => F::R16Sint,
        x if x == sys::WGPUTextureFormat_R16Float => F::R16Float,
        x if x == sys::WGPUTextureFormat_RG8Unorm => F::Rg8Unorm,
        x if x == sys::WGPUTextureFormat_RG8Snorm => F::Rg8Snorm,
        x if x == sys::WGPUTextureFormat_RG8Uint => F::Rg8Uint,
        x if x == sys::WGPUTextureFormat_RG8Sint => F::Rg8Sint,
        x if x == sys::WGPUTextureFormat_R32Uint => F::R32Uint,
        x if x == sys::WGPUTextureFormat_R32Sint => F::R32Sint,
        x if x == sys::WGPUTextureFormat_R32Float => F::R32Float,
        x if x == sys::WGPUTextureFormat_RG16Uint => F::Rg16Uint,
        x if x == sys::WGPUTextureFormat_RG16Sint => F::Rg16Sint,
        x if x == sys::WGPUTextureFormat_RG16Float => F::Rg16Float,
        x if x == sys::WGPUTextureFormat_RGBA8Unorm => F::Rgba8Unorm,
        x if x == sys::WGPUTextureFormat_RGBA8UnormSrgb => F::Rgba8UnormSrgb,
        x if x == sys::WGPUTextureFormat_RGBA8Snorm => F::Rgba8Snorm,
        x if x == sys::WGPUTextureFormat_RGBA8Uint => F::Rgba8Uint,
        x if x == sys::WGPUTextureFormat_RGBA8Sint => F::Rgba8Sint,
        x if x == sys::WGPUTextureFormat_BGRA8Unorm => F::Bgra8Unorm,
        x if x == sys::WGPUTextureFormat_BGRA8UnormSrgb => F::Bgra8UnormSrgb,
        x if x == sys::WGPUTextureFormat_RGB9E5Ufloat => F::Rgb9e5Ufloat,
        x if x == sys::WGPUTextureFormat_RGB10A2Uint => F::Rgb10a2Uint,
        x if x == sys::WGPUTextureFormat_RGB10A2Unorm => F::Rgb10a2Unorm,
        x if x == sys::WGPUTextureFormat_RG11B10Ufloat => F::Rg11b10Ufloat,
        x if x == sys::WGPUTextureFormat_RG32Uint => F::Rg32Uint,
        x if x == sys::WGPUTextureFormat_RG32Sint => F::Rg32Sint,
        x if x == sys::WGPUTextureFormat_RG32Float => F::Rg32Float,
        x if x == sys::WGPUTextureFormat_RGBA16Uint => F::Rgba16Uint,
        x if x == sys::WGPUTextureFormat_RGBA16Sint => F::Rgba16Sint,
        x if x == sys::WGPUTextureFormat_RGBA16Float => F::Rgba16Float,
        x if x == sys::WGPUTextureFormat_RGBA32Uint => F::Rgba32Uint,
        x if x == sys::WGPUTextureFormat_RGBA32Sint => F::Rgba32Sint,
        x if x == sys::WGPUTextureFormat_RGBA32Float => F::Rgba32Float,
        x if x == sys::WGPUTextureFormat_Stencil8 => F::Stencil8,
        x if x == sys::WGPUTextureFormat_Depth16Unorm => F::Depth16Unorm,
        x if x == sys::WGPUTextureFormat_Depth24Plus => F::Depth24Plus,
        x if x == sys::WGPUTextureFormat_Depth24PlusStencil8 => F::Depth24PlusStencil8,
        x if x == sys::WGPUTextureFormat_Depth32Float => F::Depth32Float,
        x if x == sys::WGPUTextureFormat_Depth32FloatStencil8 => F::Depth32FloatStencil8,
        _ => return None,
    })
}

pub(crate) fn map_buffer_usages(usage: BufferUsages) -> sys::WGPUBufferUsage {
    let mut result = 0;
    let pairs = [
        (BufferUsages::MAP_READ, sys::WGPUBufferUsage_MapRead),
        (BufferUsages::MAP_WRITE, sys::WGPUBufferUsage_MapWrite),
        (BufferUsages::COPY_SRC, sys::WGPUBufferUsage_CopySrc),
        (BufferUsages::COPY_DST, sys::WGPUBufferUsage_CopyDst),
        (BufferUsages::INDEX, sys::WGPUBufferUsage_Index),
        (BufferUsages::VERTEX, sys::WGPUBufferUsage_Vertex),
        (BufferUsages::UNIFORM, sys::WGPUBufferUsage_Uniform),
        (BufferUsages::STORAGE, sys::WGPUBufferUsage_Storage),
        (BufferUsages::INDIRECT, sys::WGPUBufferUsage_Indirect),
        (BufferUsages::QUERY_RESOLVE, sys::WGPUBufferUsage_QueryResolve),
    ];
    for (ours, theirs) in pairs {
        if usage.contains(ours) {
            result |= theirs;
        }
    }
    result
}

pub(crate) fn map_texture_usages(usage: TextureUsages) -> sys::WGPUTextureUsage {
    let mut result = 0;
    let pairs = [
        (TextureUsages::COPY_SRC, sys::WGPUTextureUsage_CopySrc),
        (TextureUsages::COPY_DST, sys::WGPUTextureUsage_CopyDst),
        (TextureUsages::TEXTURE_BINDING, sys::WGPUTextureUsage_TextureBinding),
        (TextureUsages::STORAGE_BINDING, sys::WGPUTextureUsage_StorageBinding),
        (TextureUsages::RENDER_ATTACHMENT, sys::WGPUTextureUsage_RenderAttachment),
    ];
    for (ours, theirs) in pairs {
        if usage.contains(ours) {
            result |= theirs;
        }
    }
    result
}

pub(crate) fn unmap_texture_usages(usage: sys::WGPUTextureUsage) -> TextureUsages {
    let mut result = TextureUsages::empty();
    let pairs = [
        (TextureUsages::COPY_SRC, sys::WGPUTextureUsage_CopySrc),
        (TextureUsages::COPY_DST, sys::WGPUTextureUsage_CopyDst),
        (TextureUsages::TEXTURE_BINDING, sys::WGPUTextureUsage_TextureBinding),
        (TextureUsages::STORAGE_BINDING, sys::WGPUTextureUsage_StorageBinding),
        (TextureUsages::RENDER_ATTACHMENT, sys::WGPUTextureUsage_RenderAttachment),
    ];
    for (ours, theirs) in pairs {
        if usage & theirs != 0 {
            result |= ours;
        }
    }
    result
}

pub(crate) fn map_shader_stages(stages: ShaderStages) -> sys::WGPUShaderStage {
    let mut result = 0;
    if stages.contains(ShaderStages::VERTEX) {
        result |= sys::WGPUShaderStage_Vertex;
    }
    if stages.contains(ShaderStages::FRAGMENT) {
        result |= sys::WGPUShaderStage_Fragment;
    }
    if stages.contains(ShaderStages::COMPUTE) {
        result |= sys::WGPUShaderStage_Compute;
    }
    result
}

pub(crate) fn map_color_writes(writes: ColorWrites) -> sys::WGPUColorWriteMask {
    let mut result = 0;
    let pairs = [
        (ColorWrites::RED, sys::WGPUColorWriteMask_Red),
        (ColorWrites::GREEN, sys::WGPUColorWriteMask_Green),
        (ColorWrites::BLUE, sys::WGPUColorWriteMask_Blue),
        (ColorWrites::ALPHA, sys::WGPUColorWriteMask_Alpha),
    ];
    for (ours, theirs) in pairs {
        if writes.contains(ours) {
            result |= theirs;
        }
    }
    result
}

pub(crate) fn map_extent(extent: Extent3d) -> sys::WGPUExtent3D {
    sys::WGPUExtent3D {
        width: extent.width,
        height: extent.height,
        depthOrArrayLayers: extent.depth_or_array_layers,
    }
}

pub(crate) fn map_origin(origin: Origin3d) -> sys::WGPUOrigin3D {
    sys::WGPUOrigin3D { x: origin.x, y: origin.y, z: origin.z }
}

pub(crate) fn map_color(color: Color) -> sys::WGPUColor {
    sys::WGPUColor { r: color.r, g: color.g, b: color.b, a: color.a }
}

pub(crate) fn map_texture_dimension(dim: TextureDimension) -> sys::WGPUTextureDimension {
    match dim {
        TextureDimension::D1 => sys::WGPUTextureDimension_1D,
        TextureDimension::D2 => sys::WGPUTextureDimension_2D,
        TextureDimension::D3 => sys::WGPUTextureDimension_3D,
    }
}

pub(crate) fn map_texture_view_dimension(
    dim: TextureViewDimension,
) -> sys::WGPUTextureViewDimension {
    match dim {
        TextureViewDimension::D1 => sys::WGPUTextureViewDimension_1D,
        TextureViewDimension::D2 => sys::WGPUTextureViewDimension_2D,
        TextureViewDimension::D2Array => sys::WGPUTextureViewDimension_2DArray,
        TextureViewDimension::Cube => sys::WGPUTextureViewDimension_Cube,
        TextureViewDimension::CubeArray => sys::WGPUTextureViewDimension_CubeArray,
        TextureViewDimension::D3 => sys::WGPUTextureViewDimension_3D,
    }
}

pub(crate) fn map_texture_aspect(aspect: TextureAspect) -> sys::WGPUTextureAspect {
    match aspect {
        TextureAspect::All => sys::WGPUTextureAspect_All,
        TextureAspect::StencilOnly => sys::WGPUTextureAspect_StencilOnly,
        TextureAspect::DepthOnly => sys::WGPUTextureAspect_DepthOnly,
        _ => panic!("plane aspects are not available in WebGPU"),
    }
}

pub(crate) fn map_address_mode(mode: AddressMode) -> sys::WGPUAddressMode {
    match mode {
        AddressMode::ClampToEdge => sys::WGPUAddressMode_ClampToEdge,
        AddressMode::Repeat => sys::WGPUAddressMode_Repeat,
        AddressMode::MirrorRepeat => sys::WGPUAddressMode_MirrorRepeat,
        AddressMode::ClampToBorder => panic!("ClampToBorder is not available in WebGPU"),
    }
}

pub(crate) fn map_filter_mode(mode: FilterMode) -> sys::WGPUFilterMode {
    match mode {
        FilterMode::Nearest => sys::WGPUFilterMode_Nearest,
        FilterMode::Linear => sys::WGPUFilterMode_Linear,
    }
}

pub(crate) fn map_mipmap_filter_mode(mode: MipmapFilterMode) -> sys::WGPUMipmapFilterMode {
    match mode {
        MipmapFilterMode::Nearest => sys::WGPUMipmapFilterMode_Nearest,
        MipmapFilterMode::Linear => sys::WGPUMipmapFilterMode_Linear,
    }
}

pub(crate) fn map_compare_function(func: CompareFunction) -> sys::WGPUCompareFunction {
    match func {
        CompareFunction::Never => sys::WGPUCompareFunction_Never,
        CompareFunction::Less => sys::WGPUCompareFunction_Less,
        CompareFunction::Equal => sys::WGPUCompareFunction_Equal,
        CompareFunction::LessEqual => sys::WGPUCompareFunction_LessEqual,
        CompareFunction::Greater => sys::WGPUCompareFunction_Greater,
        CompareFunction::NotEqual => sys::WGPUCompareFunction_NotEqual,
        CompareFunction::GreaterEqual => sys::WGPUCompareFunction_GreaterEqual,
        CompareFunction::Always => sys::WGPUCompareFunction_Always,
    }
}

pub(crate) fn map_primitive_topology(topology: PrimitiveTopology) -> sys::WGPUPrimitiveTopology {
    match topology {
        PrimitiveTopology::PointList => sys::WGPUPrimitiveTopology_PointList,
        PrimitiveTopology::LineList => sys::WGPUPrimitiveTopology_LineList,
        PrimitiveTopology::LineStrip => sys::WGPUPrimitiveTopology_LineStrip,
        PrimitiveTopology::TriangleList => sys::WGPUPrimitiveTopology_TriangleList,
        PrimitiveTopology::TriangleStrip => sys::WGPUPrimitiveTopology_TriangleStrip,
    }
}

pub(crate) fn map_front_face(face: FrontFace) -> sys::WGPUFrontFace {
    match face {
        FrontFace::Ccw => sys::WGPUFrontFace_CCW,
        FrontFace::Cw => sys::WGPUFrontFace_CW,
    }
}

pub(crate) fn map_cull_mode(mode: Option<Face>) -> sys::WGPUCullMode {
    match mode {
        None => sys::WGPUCullMode_None,
        Some(Face::Front) => sys::WGPUCullMode_Front,
        Some(Face::Back) => sys::WGPUCullMode_Back,
    }
}

pub(crate) fn map_index_format(format: IndexFormat) -> sys::WGPUIndexFormat {
    match format {
        IndexFormat::Uint16 => sys::WGPUIndexFormat_Uint16,
        IndexFormat::Uint32 => sys::WGPUIndexFormat_Uint32,
    }
}

pub(crate) fn map_blend_factor(factor: BlendFactor) -> sys::WGPUBlendFactor {
    match factor {
        BlendFactor::Zero => sys::WGPUBlendFactor_Zero,
        BlendFactor::One => sys::WGPUBlendFactor_One,
        BlendFactor::Src => sys::WGPUBlendFactor_Src,
        BlendFactor::OneMinusSrc => sys::WGPUBlendFactor_OneMinusSrc,
        BlendFactor::SrcAlpha => sys::WGPUBlendFactor_SrcAlpha,
        BlendFactor::OneMinusSrcAlpha => sys::WGPUBlendFactor_OneMinusSrcAlpha,
        BlendFactor::Dst => sys::WGPUBlendFactor_Dst,
        BlendFactor::OneMinusDst => sys::WGPUBlendFactor_OneMinusDst,
        BlendFactor::DstAlpha => sys::WGPUBlendFactor_DstAlpha,
        BlendFactor::OneMinusDstAlpha => sys::WGPUBlendFactor_OneMinusDstAlpha,
        BlendFactor::SrcAlphaSaturated => sys::WGPUBlendFactor_SrcAlphaSaturated,
        BlendFactor::Constant => sys::WGPUBlendFactor_Constant,
        BlendFactor::OneMinusConstant => sys::WGPUBlendFactor_OneMinusConstant,
        BlendFactor::Src1 => sys::WGPUBlendFactor_Src1,
        BlendFactor::OneMinusSrc1 => sys::WGPUBlendFactor_OneMinusSrc1,
        BlendFactor::Src1Alpha => sys::WGPUBlendFactor_Src1Alpha,
        BlendFactor::OneMinusSrc1Alpha => sys::WGPUBlendFactor_OneMinusSrc1Alpha,
    }
}

pub(crate) fn map_blend_operation(op: BlendOperation) -> sys::WGPUBlendOperation {
    match op {
        BlendOperation::Add => sys::WGPUBlendOperation_Add,
        BlendOperation::Subtract => sys::WGPUBlendOperation_Subtract,
        BlendOperation::ReverseSubtract => sys::WGPUBlendOperation_ReverseSubtract,
        BlendOperation::Min => sys::WGPUBlendOperation_Min,
        BlendOperation::Max => sys::WGPUBlendOperation_Max,
    }
}

pub(crate) fn map_blend_state(state: BlendState) -> sys::WGPUBlendState {
    sys::WGPUBlendState {
        color: sys::WGPUBlendComponent {
            operation: map_blend_operation(state.color.operation),
            srcFactor: map_blend_factor(state.color.src_factor),
            dstFactor: map_blend_factor(state.color.dst_factor),
        },
        alpha: sys::WGPUBlendComponent {
            operation: map_blend_operation(state.alpha.operation),
            srcFactor: map_blend_factor(state.alpha.src_factor),
            dstFactor: map_blend_factor(state.alpha.dst_factor),
        },
    }
}

pub(crate) fn map_stencil_operation(op: StencilOperation) -> sys::WGPUStencilOperation {
    match op {
        StencilOperation::Keep => sys::WGPUStencilOperation_Keep,
        StencilOperation::Zero => sys::WGPUStencilOperation_Zero,
        StencilOperation::Replace => sys::WGPUStencilOperation_Replace,
        StencilOperation::Invert => sys::WGPUStencilOperation_Invert,
        StencilOperation::IncrementClamp => sys::WGPUStencilOperation_IncrementClamp,
        StencilOperation::DecrementClamp => sys::WGPUStencilOperation_DecrementClamp,
        StencilOperation::IncrementWrap => sys::WGPUStencilOperation_IncrementWrap,
        StencilOperation::DecrementWrap => sys::WGPUStencilOperation_DecrementWrap,
    }
}

pub(crate) fn map_stencil_face_state(state: StencilFaceState) -> sys::WGPUStencilFaceState {
    sys::WGPUStencilFaceState {
        compare: map_compare_function(state.compare),
        failOp: map_stencil_operation(state.fail_op),
        depthFailOp: map_stencil_operation(state.depth_fail_op),
        passOp: map_stencil_operation(state.pass_op),
    }
}

pub(crate) fn map_vertex_format(format: VertexFormat) -> sys::WGPUVertexFormat {
    use VertexFormat as V;
    match format {
        V::Uint8 => sys::WGPUVertexFormat_Uint8,
        V::Uint8x2 => sys::WGPUVertexFormat_Uint8x2,
        V::Uint8x4 => sys::WGPUVertexFormat_Uint8x4,
        V::Sint8 => sys::WGPUVertexFormat_Sint8,
        V::Sint8x2 => sys::WGPUVertexFormat_Sint8x2,
        V::Sint8x4 => sys::WGPUVertexFormat_Sint8x4,
        V::Unorm8 => sys::WGPUVertexFormat_Unorm8,
        V::Unorm8x2 => sys::WGPUVertexFormat_Unorm8x2,
        V::Unorm8x4 => sys::WGPUVertexFormat_Unorm8x4,
        V::Snorm8 => sys::WGPUVertexFormat_Snorm8,
        V::Snorm8x2 => sys::WGPUVertexFormat_Snorm8x2,
        V::Snorm8x4 => sys::WGPUVertexFormat_Snorm8x4,
        V::Uint16 => sys::WGPUVertexFormat_Uint16,
        V::Uint16x2 => sys::WGPUVertexFormat_Uint16x2,
        V::Uint16x4 => sys::WGPUVertexFormat_Uint16x4,
        V::Sint16 => sys::WGPUVertexFormat_Sint16,
        V::Sint16x2 => sys::WGPUVertexFormat_Sint16x2,
        V::Sint16x4 => sys::WGPUVertexFormat_Sint16x4,
        V::Unorm16 => sys::WGPUVertexFormat_Unorm16,
        V::Unorm16x2 => sys::WGPUVertexFormat_Unorm16x2,
        V::Unorm16x4 => sys::WGPUVertexFormat_Unorm16x4,
        V::Snorm16 => sys::WGPUVertexFormat_Snorm16,
        V::Snorm16x2 => sys::WGPUVertexFormat_Snorm16x2,
        V::Snorm16x4 => sys::WGPUVertexFormat_Snorm16x4,
        V::Float16 => sys::WGPUVertexFormat_Float16,
        V::Float16x2 => sys::WGPUVertexFormat_Float16x2,
        V::Float16x4 => sys::WGPUVertexFormat_Float16x4,
        V::Float32 => sys::WGPUVertexFormat_Float32,
        V::Float32x2 => sys::WGPUVertexFormat_Float32x2,
        V::Float32x3 => sys::WGPUVertexFormat_Float32x3,
        V::Float32x4 => sys::WGPUVertexFormat_Float32x4,
        V::Uint32 => sys::WGPUVertexFormat_Uint32,
        V::Uint32x2 => sys::WGPUVertexFormat_Uint32x2,
        V::Uint32x3 => sys::WGPUVertexFormat_Uint32x3,
        V::Uint32x4 => sys::WGPUVertexFormat_Uint32x4,
        V::Sint32 => sys::WGPUVertexFormat_Sint32,
        V::Sint32x2 => sys::WGPUVertexFormat_Sint32x2,
        V::Sint32x3 => sys::WGPUVertexFormat_Sint32x3,
        V::Sint32x4 => sys::WGPUVertexFormat_Sint32x4,
        V::Unorm10_10_10_2 => sys::WGPUVertexFormat_Unorm10_10_10_2,
        V::Unorm8x4Bgra => sys::WGPUVertexFormat_Unorm8x4BGRA,
        V::Float64 | V::Float64x2 | V::Float64x3 | V::Float64x4 => {
            panic!("Float64 vertex formats are not available in WebGPU")
        }
    }
}

pub(crate) fn map_vertex_step_mode(mode: VertexStepMode) -> sys::WGPUVertexStepMode {
    match mode {
        VertexStepMode::Vertex => sys::WGPUVertexStepMode_Vertex,
        VertexStepMode::Instance => sys::WGPUVertexStepMode_Instance,
    }
}

pub(crate) fn map_load_op<V>(op: &LoadOp<V>) -> sys::WGPULoadOp {
    match op {
        LoadOp::Clear(_) => sys::WGPULoadOp_Clear,
        LoadOp::Load => sys::WGPULoadOp_Load,
        LoadOp::DontCare(_) => sys::WGPULoadOp_Load,
    }
}

pub(crate) fn map_store_op(op: StoreOp) -> sys::WGPUStoreOp {
    match op {
        StoreOp::Store => sys::WGPUStoreOp_Store,
        StoreOp::Discard => sys::WGPUStoreOp_Discard,
    }
}

pub(crate) fn map_present_mode(mode: PresentMode) -> sys::WGPUPresentMode {
    match mode {
        PresentMode::AutoVsync | PresentMode::Fifo | PresentMode::FifoRelaxed => {
            sys::WGPUPresentMode_Fifo
        }
        PresentMode::AutoNoVsync | PresentMode::Immediate => sys::WGPUPresentMode_Immediate,
        PresentMode::Mailbox => sys::WGPUPresentMode_Mailbox,
    }
}

pub(crate) fn unmap_present_mode(mode: sys::WGPUPresentMode) -> PresentMode {
    match mode {
        x if x == sys::WGPUPresentMode_Immediate => PresentMode::Immediate,
        x if x == sys::WGPUPresentMode_Mailbox => PresentMode::Mailbox,
        x if x == sys::WGPUPresentMode_FifoRelaxed => PresentMode::FifoRelaxed,
        _ => PresentMode::Fifo,
    }
}

pub(crate) fn map_alpha_mode(mode: CompositeAlphaMode) -> sys::WGPUCompositeAlphaMode {
    match mode {
        CompositeAlphaMode::Auto => sys::WGPUCompositeAlphaMode_Auto,
        CompositeAlphaMode::Opaque => sys::WGPUCompositeAlphaMode_Opaque,
        CompositeAlphaMode::PreMultiplied => sys::WGPUCompositeAlphaMode_Premultiplied,
        CompositeAlphaMode::PostMultiplied => sys::WGPUCompositeAlphaMode_Unpremultiplied,
        CompositeAlphaMode::Inherit => sys::WGPUCompositeAlphaMode_Inherit,
    }
}

pub(crate) fn unmap_alpha_mode(mode: sys::WGPUCompositeAlphaMode) -> CompositeAlphaMode {
    match mode {
        x if x == sys::WGPUCompositeAlphaMode_Opaque => CompositeAlphaMode::Opaque,
        x if x == sys::WGPUCompositeAlphaMode_Premultiplied => CompositeAlphaMode::PreMultiplied,
        x if x == sys::WGPUCompositeAlphaMode_Unpremultiplied => CompositeAlphaMode::PostMultiplied,
        x if x == sys::WGPUCompositeAlphaMode_Inherit => CompositeAlphaMode::Inherit,
        _ => CompositeAlphaMode::Auto,
    }
}

pub(crate) fn map_map_mode(mode: MapMode) -> sys::WGPUMapMode {
    match mode {
        MapMode::Read => sys::WGPUMapMode_Read,
        MapMode::Write => sys::WGPUMapMode_Write,
    }
}

pub(crate) fn map_error_filter(filter: ErrorFilter) -> sys::WGPUErrorFilter {
    match filter {
        ErrorFilter::Validation => sys::WGPUErrorFilter_Validation,
        ErrorFilter::OutOfMemory => sys::WGPUErrorFilter_OutOfMemory,
        ErrorFilter::Internal => sys::WGPUErrorFilter_Internal,
    }
}

/// Feature mapping between the Rust bitflags and `WGPUFeatureName` values.
pub(crate) const FEATURE_PAIRS: &[(Features, sys::WGPUFeatureName)] = &[
    (Features::DEPTH_CLIP_CONTROL, sys::WGPUFeatureName_DepthClipControl),
    (Features::DEPTH32FLOAT_STENCIL8, sys::WGPUFeatureName_Depth32FloatStencil8),
    (Features::TEXTURE_COMPRESSION_BC, sys::WGPUFeatureName_TextureCompressionBC),
    (Features::TEXTURE_COMPRESSION_BC_SLICED_3D, sys::WGPUFeatureName_TextureCompressionBCSliced3D),
    (Features::TEXTURE_COMPRESSION_ETC2, sys::WGPUFeatureName_TextureCompressionETC2),
    (Features::TEXTURE_COMPRESSION_ASTC, sys::WGPUFeatureName_TextureCompressionASTC),
    (
        Features::TEXTURE_COMPRESSION_ASTC_SLICED_3D,
        sys::WGPUFeatureName_TextureCompressionASTCSliced3D,
    ),
    (Features::TIMESTAMP_QUERY, sys::WGPUFeatureName_TimestampQuery),
    (Features::INDIRECT_FIRST_INSTANCE, sys::WGPUFeatureName_IndirectFirstInstance),
    (Features::SHADER_F16, sys::WGPUFeatureName_ShaderF16),
    (Features::RG11B10UFLOAT_RENDERABLE, sys::WGPUFeatureName_RG11B10UfloatRenderable),
    (Features::BGRA8UNORM_STORAGE, sys::WGPUFeatureName_BGRA8UnormStorage),
    (Features::FLOAT32_FILTERABLE, sys::WGPUFeatureName_Float32Filterable),
    (Features::FLOAT32_BLENDABLE, sys::WGPUFeatureName_Float32Blendable),
    (Features::CLIP_DISTANCES, sys::WGPUFeatureName_ClipDistances),
    (Features::DUAL_SOURCE_BLENDING, sys::WGPUFeatureName_DualSourceBlending),
    (Features::PRIMITIVE_INDEX, sys::WGPUFeatureName_PrimitiveIndex),
];

pub(crate) fn map_features(features: Features) -> Vec<sys::WGPUFeatureName> {
    FEATURE_PAIRS
        .iter()
        .filter(|(ours, _)| features.contains(*ours))
        .map(|&(_, theirs)| theirs)
        .collect()
}

pub(crate) fn unmap_features(names: &[sys::WGPUFeatureName]) -> Features {
    let mut features = Features::empty();
    for name in names {
        if let Some((ours, _)) = FEATURE_PAIRS.iter().find(|(_, theirs)| theirs == name) {
            features |= *ours;
        }
    }
    features
}

pub(crate) fn map_limits(limits: &Limits) -> sys::WGPULimits {
    let mut out: sys::WGPULimits = unsafe { std::mem::zeroed() };
    out.maxTextureDimension1D = limits.max_texture_dimension_1d;
    out.maxTextureDimension2D = limits.max_texture_dimension_2d;
    out.maxTextureDimension3D = limits.max_texture_dimension_3d;
    out.maxTextureArrayLayers = limits.max_texture_array_layers;
    out.maxBindGroups = limits.max_bind_groups;
    out.maxBindingsPerBindGroup = limits.max_bindings_per_bind_group;
    out.maxDynamicUniformBuffersPerPipelineLayout =
        limits.max_dynamic_uniform_buffers_per_pipeline_layout;
    out.maxDynamicStorageBuffersPerPipelineLayout =
        limits.max_dynamic_storage_buffers_per_pipeline_layout;
    out.maxSampledTexturesPerShaderStage = limits.max_sampled_textures_per_shader_stage;
    out.maxSamplersPerShaderStage = limits.max_samplers_per_shader_stage;
    out.maxStorageBuffersPerShaderStage = limits.max_storage_buffers_per_shader_stage;
    out.maxStorageTexturesPerShaderStage = limits.max_storage_textures_per_shader_stage;
    out.maxUniformBuffersPerShaderStage = limits.max_uniform_buffers_per_shader_stage;
    out.maxUniformBufferBindingSize = limits.max_uniform_buffer_binding_size;
    out.maxStorageBufferBindingSize = limits.max_storage_buffer_binding_size;
    out.minUniformBufferOffsetAlignment = limits.min_uniform_buffer_offset_alignment;
    out.minStorageBufferOffsetAlignment = limits.min_storage_buffer_offset_alignment;
    out.maxVertexBuffers = limits.max_vertex_buffers;
    out.maxBufferSize = limits.max_buffer_size;
    out.maxVertexAttributes = limits.max_vertex_attributes;
    out.maxVertexBufferArrayStride = limits.max_vertex_buffer_array_stride;
    out.maxInterStageShaderVariables = limits.max_inter_stage_shader_variables;
    out.maxColorAttachments = limits.max_color_attachments;
    out.maxColorAttachmentBytesPerSample = limits.max_color_attachment_bytes_per_sample;
    out.maxComputeWorkgroupStorageSize = limits.max_compute_workgroup_storage_size;
    out.maxComputeInvocationsPerWorkgroup = limits.max_compute_invocations_per_workgroup;
    out.maxComputeWorkgroupSizeX = limits.max_compute_workgroup_size_x;
    out.maxComputeWorkgroupSizeY = limits.max_compute_workgroup_size_y;
    out.maxComputeWorkgroupSizeZ = limits.max_compute_workgroup_size_z;
    out.maxComputeWorkgroupsPerDimension = limits.max_compute_workgroups_per_dimension;
    out
}

pub(crate) fn unmap_limits(limits: &sys::WGPULimits) -> Limits {
    let mut out = Limits::defaults();
    out.max_texture_dimension_1d = limits.maxTextureDimension1D;
    out.max_texture_dimension_2d = limits.maxTextureDimension2D;
    out.max_texture_dimension_3d = limits.maxTextureDimension3D;
    out.max_texture_array_layers = limits.maxTextureArrayLayers;
    out.max_bind_groups = limits.maxBindGroups;
    out.max_bindings_per_bind_group = limits.maxBindingsPerBindGroup;
    out.max_dynamic_uniform_buffers_per_pipeline_layout =
        limits.maxDynamicUniformBuffersPerPipelineLayout;
    out.max_dynamic_storage_buffers_per_pipeline_layout =
        limits.maxDynamicStorageBuffersPerPipelineLayout;
    out.max_sampled_textures_per_shader_stage = limits.maxSampledTexturesPerShaderStage;
    out.max_samplers_per_shader_stage = limits.maxSamplersPerShaderStage;
    out.max_storage_buffers_per_shader_stage = limits.maxStorageBuffersPerShaderStage;
    out.max_storage_textures_per_shader_stage = limits.maxStorageTexturesPerShaderStage;
    out.max_uniform_buffers_per_shader_stage = limits.maxUniformBuffersPerShaderStage;
    out.max_uniform_buffer_binding_size = limits.maxUniformBufferBindingSize;
    out.max_storage_buffer_binding_size = limits.maxStorageBufferBindingSize;
    out.min_uniform_buffer_offset_alignment = limits.minUniformBufferOffsetAlignment;
    out.min_storage_buffer_offset_alignment = limits.minStorageBufferOffsetAlignment;
    out.max_vertex_buffers = limits.maxVertexBuffers;
    out.max_buffer_size = limits.maxBufferSize;
    out.max_vertex_attributes = limits.maxVertexAttributes;
    out.max_vertex_buffer_array_stride = limits.maxVertexBufferArrayStride;
    out.max_inter_stage_shader_variables = limits.maxInterStageShaderVariables;
    out.max_color_attachments = limits.maxColorAttachments;
    out.max_color_attachment_bytes_per_sample = limits.maxColorAttachmentBytesPerSample;
    out.max_compute_workgroup_storage_size = limits.maxComputeWorkgroupStorageSize;
    out.max_compute_invocations_per_workgroup = limits.maxComputeInvocationsPerWorkgroup;
    out.max_compute_workgroup_size_x = limits.maxComputeWorkgroupSizeX;
    out.max_compute_workgroup_size_y = limits.maxComputeWorkgroupSizeY;
    out.max_compute_workgroup_size_z = limits.maxComputeWorkgroupSizeZ;
    out.max_compute_workgroups_per_dimension = limits.maxComputeWorkgroupsPerDimension;
    out
}
