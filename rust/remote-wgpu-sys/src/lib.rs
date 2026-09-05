//! Raw FFI bindings to the `remote_webgpu` C library: the standard
//! `webgpu.h` API (generated, see `tools/generate_ffi.py`) plus the
//! `webgpu/remote.h` transport extensions (hand-written, small).

mod ffi;
pub mod remote;
#[cfg(test)]
mod size_check;

pub use ffi::*;

include!(concat!(env!("OUT_DIR"), "/protocol_version.rs"));
