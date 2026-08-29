//! A drop-in replacement for the `wgpu` crate whose GPU lives on the other
//! end of a websocket: every call is forwarded through the `remote_webgpu` C
//! library to a browser client exposing `navigator.gpu`.
//!
//! Only the API surface exercised by the upstream wgpu examples is
//! implemented; see `../../README.md` and `run_wgpu_example.sh` at the repo
//! root for how to run those examples against this crate.

mod api;
mod convert;
mod limits;
mod types;

pub mod util;

pub use api::*;
pub use types::*;

/// Creates a [`ShaderModuleDescriptor`] from a WGSL file path relative to
/// the calling file.
#[macro_export]
macro_rules! include_wgsl {
    ($($token:tt)*) => {
        $crate::ShaderModuleDescriptor {
            label: Some($($token)*),
            source: $crate::ShaderSource::Wgsl(include_str!($($token)*).into()),
        }
    };
}

/// Macro to produce an array of [`VertexAttribute`].
///
/// Output has type: `[VertexAttribute; _]`. Usage is as follows:
/// ```
/// # use wgpu::vertex_attr_array;
/// let attrs = vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Uint16x4];
/// ```
#[macro_export]
macro_rules! vertex_attr_array {
    ($($location:expr => $format:ident),* $(,)?) => {
        $crate::_vertex_attr_array_helper!([] ; 0; $($location => $format ,)*)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _vertex_attr_array_helper {
    ([$($t:expr,)*] ; $off:expr ;) => { [$($t,)*] };
    ([$($t:expr,)*] ; $off:expr ; $location:expr => $format:ident, $($ll:expr => $ii:ident ,)*) => {
        $crate::_vertex_attr_array_helper!(
            [$($t,)*
             $crate::VertexAttribute {
                 format: $crate::VertexFormat::$format,
                 offset: $off,
                 shader_location: $location,
             },];
            $off + $crate::VertexFormat::$format.size();
            $($ll => $ii ,)*
        )
    };
}
