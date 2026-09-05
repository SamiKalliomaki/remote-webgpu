//! Helpers for this crate's unit tests.

use alloc::sync::Arc;
use bevy_platform::future::block_on;
use wgpu::{
    BackendOptions, Backends, DeviceDescriptor, Instance, InstanceDescriptor, InstanceFlags,
    NoopBackendOptions, RequestAdapterOptions,
};

use crate::renderer::{RenderDevice, RenderQueue, WgpuWrapper};

/// Creates a dummy [`RenderDevice`] and [`RenderQueue`] on `wgpu`'s noop backend.
///
/// This lets tests exercise real `wgpu` resource creation without requiring a
/// GPU adapter, so they can run in headless environments.
///
/// remote-webgpu: **this does not work in this workspace.** `wgpu` here is the
/// drop-in shim over `remote_webgpu`, which has no noop backend; every adapter
/// belongs to a connected browser tab, so `Instance::new` panics with
/// `no port configured` unless a runtime and a client are up. The upstream
/// tests that call this are marked `#[ignore]` rather than deleted, since they
/// are the vendored fork's only coverage of slab and mesh allocator behaviour.
///
/// Making them run means giving this function a fake client instead of a noop
/// adapter: start the runtime on an ephemeral port and complete a handshake,
/// the way `remote-wgpu-runtime/tests/support/fake_client.rs` does for the
/// lifetime tests. Share one device across the whole test binary, because the
/// runtime hands out one client per connection.
pub fn create_dummy_device() -> (RenderDevice, RenderQueue) {
    let instance = Instance::new(InstanceDescriptor {
        backends: Backends::NOOP,
        flags: InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        display: None,
        backend_options: BackendOptions {
            noop: NoopBackendOptions { enable: true },
            ..Default::default()
        },
    });

    let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("the noop backend should always produce an adapter");
    let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("the noop backend should always produce a device");

    (
        RenderDevice::from(device),
        RenderQueue(Arc::new(WgpuWrapper::new(queue))),
    )
}
