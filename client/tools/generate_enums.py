#!/usr/bin/env python3
"""Generate src/gen/enums.ts from the server's webgpu.h.

The wire protocol carries the numeric webgpu.h enum values; the client
needs the corresponding WebGPU (JavaScript) string names.  This script
parses the enum blocks out of webgpu.h and matches every entry against the
official JS name lists below by normalization (lowercase, alphanumerics
only), which is unambiguous in both directions.  Entries with no JS
equivalent (Undefined, Force32, BindingNotUsed, ...) are skipped.

Run from the client directory:

    python3 tools/generate_enums.py
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEADER = ROOT.parent / "remote_webgpu" / "include" / "webgpu" / "webgpu.h"
OUTPUT = ROOT / "src" / "gen" / "enums.ts"

# (C enum name, TS export name, TS value type, official JS names)
ENUMS = [
    ("WGPUAddressMode", "ADDRESS_MODES", "GPUAddressMode",
     ["clamp-to-edge", "repeat", "mirror-repeat"]),
    ("WGPUBlendFactor", "BLEND_FACTORS", "GPUBlendFactor",
     ["zero", "one", "src", "one-minus-src", "src-alpha",
      "one-minus-src-alpha", "dst", "one-minus-dst", "dst-alpha",
      "one-minus-dst-alpha", "src-alpha-saturated", "constant",
      "one-minus-constant", "src1", "one-minus-src1", "src1-alpha",
      "one-minus-src1-alpha"]),
    ("WGPUBlendOperation", "BLEND_OPERATIONS", "GPUBlendOperation",
     ["add", "subtract", "reverse-subtract", "min", "max"]),
    ("WGPUBufferBindingType", "BUFFER_BINDING_TYPES", "GPUBufferBindingType",
     ["uniform", "storage", "read-only-storage"]),
    ("WGPUCompareFunction", "COMPARE_FUNCTIONS", "GPUCompareFunction",
     ["never", "less", "equal", "less-equal", "greater", "not-equal",
      "greater-equal", "always"]),
    ("WGPUCompositeAlphaMode", "ALPHA_MODES", "GPUCanvasAlphaMode",
     ["opaque", "premultiplied"]),
    ("WGPUCullMode", "CULL_MODES", "GPUCullMode",
     ["none", "front", "back"]),
    ("WGPUErrorFilter", "ERROR_FILTERS", "GPUErrorFilter",
     ["validation", "out-of-memory", "internal"]),
    ("WGPUFeatureName", "FEATURE_NAMES", "string",
     ["core-features-and-limits", "depth-clip-control",
      "depth32float-stencil8", "texture-compression-bc",
      "texture-compression-bc-sliced-3d", "texture-compression-etc2",
      "texture-compression-astc", "texture-compression-astc-sliced-3d",
      "timestamp-query", "indirect-first-instance", "shader-f16",
      "rg11b10ufloat-renderable", "bgra8unorm-storage", "float32-filterable",
      "float32-blendable", "clip-distances", "dual-source-blending",
      "subgroups", "texture-formats-tier1", "texture-formats-tier2",
      "primitive-index", "texture-component-swizzle"]),
    ("WGPUFilterMode", "FILTER_MODES", "GPUFilterMode",
     ["nearest", "linear"]),
    ("WGPUMipmapFilterMode", "MIPMAP_FILTER_MODES", "GPUMipmapFilterMode",
     ["nearest", "linear"]),
    ("WGPUFrontFace", "FRONT_FACES", "GPUFrontFace",
     ["ccw", "cw"]),
    ("WGPUIndexFormat", "INDEX_FORMATS", "GPUIndexFormat",
     ["uint16", "uint32"]),
    ("WGPULoadOp", "LOAD_OPS", "GPULoadOp",
     ["load", "clear"]),
    ("WGPUStoreOp", "STORE_OPS", "GPUStoreOp",
     ["store", "discard"]),
    ("WGPUPrimitiveTopology", "TOPOLOGIES", "GPUPrimitiveTopology",
     ["point-list", "line-list", "line-strip", "triangle-list",
      "triangle-strip"]),
    ("WGPUQueryType", "QUERY_TYPES", "GPUQueryType",
     ["occlusion", "timestamp"]),
    ("WGPUSamplerBindingType", "SAMPLER_BINDING_TYPES", "GPUSamplerBindingType",
     ["filtering", "non-filtering", "comparison"]),
    ("WGPUStencilOperation", "STENCIL_OPERATIONS", "GPUStencilOperation",
     ["keep", "zero", "replace", "invert", "increment-clamp",
      "decrement-clamp", "increment-wrap", "decrement-wrap"]),
    ("WGPUStorageTextureAccess", "STORAGE_TEXTURE_ACCESSES", "GPUStorageTextureAccess",
     ["write-only", "read-only", "read-write"]),
    ("WGPUTextureAspect", "TEXTURE_ASPECTS", "GPUTextureAspect",
     ["all", "stencil-only", "depth-only"]),
    ("WGPUTextureDimension", "TEXTURE_DIMENSIONS", "GPUTextureDimension",
     ["1d", "2d", "3d"]),
    ("WGPUTextureViewDimension", "TEXTURE_VIEW_DIMENSIONS", "GPUTextureViewDimension",
     ["1d", "2d", "2d-array", "cube", "cube-array", "3d"]),
    ("WGPUTextureSampleType", "TEXTURE_SAMPLE_TYPES", "GPUTextureSampleType",
     ["float", "unfilterable-float", "depth", "sint", "uint"]),
    ("WGPUTextureFormat", "TEXTURE_FORMATS", "GPUTextureFormat",
     ["r8unorm", "r8snorm", "r8uint", "r8sint",
      "r16unorm", "r16snorm", "r16uint", "r16sint", "r16float",
      "rg8unorm", "rg8snorm", "rg8uint", "rg8sint",
      "r32float", "r32uint", "r32sint",
      "rg16unorm", "rg16snorm", "rg16uint", "rg16sint", "rg16float",
      "rgba8unorm", "rgba8unorm-srgb", "rgba8snorm", "rgba8uint",
      "rgba8sint", "bgra8unorm", "bgra8unorm-srgb",
      "rgb10a2uint", "rgb10a2unorm", "rg11b10ufloat", "rgb9e5ufloat",
      "rg32float", "rg32uint", "rg32sint",
      "rgba16unorm", "rgba16snorm", "rgba16uint", "rgba16sint",
      "rgba16float", "rgba32float", "rgba32uint", "rgba32sint",
      "stencil8", "depth16unorm", "depth24plus", "depth24plus-stencil8",
      "depth32float", "depth32float-stencil8",
      "bc1-rgba-unorm", "bc1-rgba-unorm-srgb",
      "bc2-rgba-unorm", "bc2-rgba-unorm-srgb",
      "bc3-rgba-unorm", "bc3-rgba-unorm-srgb",
      "bc4-r-unorm", "bc4-r-snorm", "bc5-rg-unorm", "bc5-rg-snorm",
      "bc6h-rgb-ufloat", "bc6h-rgb-float",
      "bc7-rgba-unorm", "bc7-rgba-unorm-srgb",
      "etc2-rgb8unorm", "etc2-rgb8unorm-srgb",
      "etc2-rgb8a1unorm", "etc2-rgb8a1unorm-srgb",
      "etc2-rgba8unorm", "etc2-rgba8unorm-srgb",
      "eac-r11unorm", "eac-r11snorm", "eac-rg11unorm", "eac-rg11snorm",
      "astc-4x4-unorm", "astc-4x4-unorm-srgb",
      "astc-5x4-unorm", "astc-5x4-unorm-srgb",
      "astc-5x5-unorm", "astc-5x5-unorm-srgb",
      "astc-6x5-unorm", "astc-6x5-unorm-srgb",
      "astc-6x6-unorm", "astc-6x6-unorm-srgb",
      "astc-8x5-unorm", "astc-8x5-unorm-srgb",
      "astc-8x6-unorm", "astc-8x6-unorm-srgb",
      "astc-8x8-unorm", "astc-8x8-unorm-srgb",
      "astc-10x5-unorm", "astc-10x5-unorm-srgb",
      "astc-10x6-unorm", "astc-10x6-unorm-srgb",
      "astc-10x8-unorm", "astc-10x8-unorm-srgb",
      "astc-10x10-unorm", "astc-10x10-unorm-srgb",
      "astc-12x10-unorm", "astc-12x10-unorm-srgb",
      "astc-12x12-unorm", "astc-12x12-unorm-srgb"]),
    ("WGPUVertexFormat", "VERTEX_FORMATS", "GPUVertexFormat",
     ["uint8", "uint8x2", "uint8x4", "sint8", "sint8x2", "sint8x4",
      "unorm8", "unorm8x2", "unorm8x4", "snorm8", "snorm8x2", "snorm8x4",
      "uint16", "uint16x2", "uint16x4", "sint16", "sint16x2", "sint16x4",
      "unorm16", "unorm16x2", "unorm16x4", "snorm16", "snorm16x2",
      "snorm16x4", "float16", "float16x2", "float16x4",
      "float32", "float32x2", "float32x3", "float32x4",
      "uint32", "uint32x2", "uint32x3", "uint32x4",
      "sint32", "sint32x2", "sint32x3", "sint32x4",
      "unorm10-10-10-2", "unorm8x4-bgra"]),
    ("WGPUVertexStepMode", "STEP_MODES", "GPUVertexStepMode",
     ["vertex", "instance"]),
    ("WGPUWGSLLanguageFeatureName", "WGSL_LANGUAGE_FEATURES", "string",
     ["readonly_and_readwrite_storage_textures",
      "packed_4x8_integer_dot_product", "unrestricted_pointer_parameters",
      "pointer_composite_access", "uniform_buffer_standard_layout",
      "subgroup_id", "texture_and_sampler_let", "subgroup_uniformity",
      "texture_formats_tier1", "linear_indexing"]),
]

# Entries that legitimately have no JS name.
NO_JS = {"Undefined", "Force32", "BindingNotUsed", "Auto", "Inherit",
         "Unpremultiplied"}


def normalize(name: str) -> str:
    return re.sub(r"[^a-z0-9]", "", name.lower())


def main() -> int:
    src = HEADER.read_text()
    blocks = {
        m.group(1): re.findall(r"%s_(\w+) = (0x[0-9A-Fa-f]+|\d+)" % m.group(1),
                               m.group(2))
        for m in re.finditer(r"typedef enum (\w+) \{(.*?)\} \1", src, re.S)
    }

    out = [
        "/* Generated by tools/generate_enums.py from webgpu.h -- do not edit. */",
        "",
        "/* Numeric webgpu.h enum values -> WebGPU (JS) string names. */",
        "",
    ]
    ok = True
    for c_name, ts_name, ts_type, js_names in ENUMS:
        js_by_norm = {normalize(j): j for j in js_names}
        if len(js_by_norm) != len(js_names):
            print(f"{c_name}: duplicate normalized JS names", file=sys.stderr)
            ok = False
        entries = []
        matched = set()
        for entry, value in blocks[c_name]:
            if entry == "Force32":
                continue
            js = js_by_norm.get(normalize(entry))
            if js is None:
                if entry not in NO_JS:
                    print(f"{c_name}_{entry}: no JS name; skipped", file=sys.stderr)
                continue
            entries.append((int(value, 0), js))
            matched.add(js)
        for js in js_names:
            if js not in matched:
                print(f"{c_name}: JS name \"{js}\" matched no C entry", file=sys.stderr)
                ok = False
        out.append(f"export const {ts_name}: Record<number, {ts_type}> = {{")
        for value, js in entries:
            out.append(f'  {value}: "{js}",')
        out.append("};")
        out.append("")

    out += [
        "function invert(table: Record<number, string>): Record<string, number> {",
        "  const out: Record<string, number> = {};",
        "  for (const [value, name] of Object.entries(table)) out[name] = Number(value);",
        "  return out;",
        "}",
        "",
        "/* JS names -> numeric webgpu.h values, for reporting client",
        " * capabilities to the server. */",
        "export const FEATURE_NAME_VALUES = invert(FEATURE_NAMES);",
        "export const WGSL_LANGUAGE_FEATURE_VALUES = invert(WGSL_LANGUAGE_FEATURES);",
        "",
    ]

    OUTPUT.write_text("\n".join(out))
    print(f"wrote {OUTPUT}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
